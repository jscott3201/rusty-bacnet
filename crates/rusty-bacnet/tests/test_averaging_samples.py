"""Installed-artifact tests for the Averaging sample route (#1083, #1092).

`add_averaging_sample_local` feeds a running Averaging object the samples the
application took, and the statistics it changes go through the server's COV
path. The statistics cover a window of the most recent Window_Samples
attempts, and a write of a window row empties it.
"""

from __future__ import annotations

import ast
import asyncio
import inspect
import math
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetProtocolError,
    ErrorClass,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)


COUNTER_TIMEOUT = 2.0
SILENCE_TIMEOUT = 0.25
AVG = ObjectIdentifier(ObjectType.AVERAGING, 1)
AV = ObjectIdentifier(ObjectType.ANALOG_VALUE, 2)


def installed_stub_method(name: str) -> ast.FunctionDef:
    stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    tree = ast.parse(stub_path.read_text(encoding="utf-8"), filename=str(stub_path))
    server = next(
        node
        for node in tree.body
        if isinstance(node, ast.ClassDef) and node.name == "BACnetServer"
    )
    return next(
        node
        for node in server.body
        if isinstance(node, ast.FunctionDef) and node.name == name
    )


def make_server() -> BACnetServer:
    server = BACnetServer(
        device_instance=1_083_001,
        device_name="Averaging Sample Artifact Test",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )
    server.add_averaging(1, "AVG-1")
    server.add_analog_value(2, "AV-2")
    return server


def assert_protocol_error(
    case: unittest.TestCase,
    raised: BacnetProtocolError,
    error_class: ErrorClass,
    error_code: ErrorCode,
) -> None:
    case.assertEqual(raised.error_class, error_class.to_raw())
    case.assertEqual(raised.error_code, error_code.to_raw())


class AveragingStubContractTests(unittest.TestCase):
    def test_runtime_and_stub_expose_the_sample_route(self) -> None:
        runtime = list(
            inspect.signature(BACnetServer.add_averaging_sample_local).parameters
        )
        self.assertEqual(runtime, ["self", "object_id", "value"])
        method = installed_stub_method("add_averaging_sample_local")
        self.assertEqual(
            [argument.arg for argument in method.args.args],
            ["self", "object_id", "value"],
        )
        self.assertEqual(ast.unparse(method.returns), "Awaitable[None]")
        docs = ast.get_docstring(method)
        assert docs is not None
        for phrase in (
            "Object_Property_Reference",
            "INVALID_DATA_TYPE",
            "SubscribeCOVProperty",
        ):
            with self.subTest(documented=phrase):
                self.assertIn(phrase, docs)


class AveragingLiveServerTests(unittest.TestCase):
    def test_live_server_takes_samples_and_reports_them(self) -> None:
        asyncio.run(self._exercise_live_server())

    async def _exercise_live_server(self) -> None:
        server = make_server()
        with self.assertRaises(RuntimeError):
            await server.add_averaging_sample_local(AVG, PropertyValue.real(1.0))

        await server.start()
        try:
            for sample in (
                PropertyValue.real(2.5),
                PropertyValue.unsigned(9),
                PropertyValue.signed(-4),
                PropertyValue.enumerated(3),
                PropertyValue.boolean(True),
                PropertyValue.boolean(False),
            ):
                with self.subTest(sample=sample):
                    self.assertIsNone(
                        await server.add_averaging_sample_local(AVG, sample)
                    )
            self.assertEqual(
                await self._read(server, PropertyIdentifier.MINIMUM_VALUE),
                PropertyValue.real(-4.0),
            )
            self.assertEqual(
                await self._read(server, PropertyIdentifier.MAXIMUM_VALUE),
                PropertyValue.real(9.0),
            )
            average = await self._read(server, PropertyIdentifier.AVERAGE_VALUE)
            self.assertAlmostEqual(average.value, 11.5 / 6, places=5)
            await self._assert_counts(server, 6)

            for value, code in (
                (PropertyValue.double(1.0), ErrorCode.INVALID_DATA_TYPE),
                (PropertyValue.character_string("7"), ErrorCode.INVALID_DATA_TYPE),
                (PropertyValue.real(float("nan")), ErrorCode.VALUE_OUT_OF_RANGE),
            ):
                with self.subTest(refused=value):
                    with self.assertRaises(BacnetProtocolError) as raised:
                        await server.add_averaging_sample_local(AVG, value)
                    assert_protocol_error(
                        self, raised.exception, ErrorClass.PROPERTY, code
                    )
            for target, code in (
                (AV, ErrorCode.OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED),
                (ObjectIdentifier(ObjectType.AVERAGING, 9), ErrorCode.UNKNOWN_OBJECT),
            ):
                with self.subTest(target=target):
                    with self.assertRaises(BacnetProtocolError) as raised:
                        await server.add_averaging_sample_local(
                            target, PropertyValue.real(1.0)
                        )
                    assert_protocol_error(
                        self, raised.exception, ErrorClass.OBJECT, code
                    )
            await self._assert_counts(server, 6)

            await self._exercise_cov(server)
        finally:
            await server.stop()

    async def _exercise_cov(self, server: BACnetServer) -> None:
        address = await server.local_address()
        async with BACnetClient(
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
            apdu_timeout_ms=2_000,
        ) as client:
            # Table 13-1 has no Averaging row, so there is no SubscribeCOV.
            with self.assertRaises(BacnetProtocolError) as raised:
                await client.subscribe_cov(
                    address,
                    subscriber_process_identifier=1083,
                    monitored_object_identifier=AVG,
                    confirmed=False,
                    lifetime=60,
                )
            assert_protocol_error(
                self,
                raised.exception,
                ErrorClass.OBJECT,
                ErrorCode.OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            )

            # A property subscription is admitted; the server's counters
            # show its COV-multiple notifications.
            sent = (await server.cov_counters())["notifications_sent"]
            await client.subscribe_cov_property_multiple(
                address,
                1083,
                [(AVG, [(PropertyIdentifier.AVERAGE_VALUE, None, None, False)])],
                False,
                max_notification_delay=10,
                lifetime=60,
            )
            sent = await self._wait_for_sent(server, sent + 1)  # initial report
            await server.add_averaging_sample_local(AVG, PropertyValue.real(100.0))
            sent = await self._wait_for_sent(server, sent + 1)

            # A sample equal to the average leaves it unchanged: no report.
            average = await self._read(server, PropertyIdentifier.AVERAGE_VALUE)
            await server.add_averaging_sample_local(AVG, average)
            await self._assert_counts(server, 8)
            await asyncio.sleep(SILENCE_TIMEOUT)
            self.assertEqual(
                (await server.cov_counters())["notifications_sent"], sent
            )

    @staticmethod
    async def _read(server: BACnetServer, property_id: object) -> PropertyValue:
        return await server.read_property(AVG, property_id)

    async def _assert_counts(self, server: BACnetServer, count: int) -> None:
        for property_id in (
            PropertyIdentifier.ATTEMPTED_SAMPLES,
            PropertyIdentifier.VALID_SAMPLES,
        ):
            self.assertEqual(
                await self._read(server, property_id), PropertyValue.unsigned(count)
            )

    async def _wait_for_sent(self, server: BACnetServer, target: int) -> int:
        async def reached() -> int:
            while True:
                sent = (await server.cov_counters())["notifications_sent"]
                if sent >= target:
                    return sent
                await asyncio.sleep(0.01)

        sent = await asyncio.wait_for(reached(), timeout=COUNTER_TIMEOUT)
        self.assertEqual(sent, target)
        return sent


class AveragingWindowTests(unittest.TestCase):
    def test_runtime_and_stub_expose_the_window_keywords(self) -> None:
        parameters = inspect.signature(BACnetServer.add_averaging).parameters
        self.assertEqual(
            list(parameters),
            ["self", "instance", "name", "window_interval", "window_samples"],
        )
        for keyword in ("window_interval", "window_samples"):
            with self.subTest(keyword=keyword):
                self.assertIs(parameters[keyword].kind, inspect.Parameter.KEYWORD_ONLY)
                self.assertIsNone(parameters[keyword].default)
        method = installed_stub_method("add_averaging")
        self.assertEqual(
            [argument.arg for argument in method.args.args],
            ["self", "instance", "name"],
        )
        self.assertEqual(
            [argument.arg for argument in method.args.kwonlyargs],
            ["window_interval", "window_samples"],
        )
        sample = installed_stub_method("add_averaging_sample_local")
        self.assertEqual(
            ast.unparse(sample.args.args[2].annotation), "PropertyValue | None"
        )

    def test_out_of_range_windows_are_refused_at_registration(self) -> None:
        server = make_server()
        for keywords in (
            {"window_samples": 0},
            {"window_samples": 1441},
            {"window_interval": 0},
        ):
            with self.subTest(**keywords):
                with self.assertRaises(BacnetProtocolError) as raised:
                    server.add_averaging(5, "AVG-5", **keywords)
                assert_protocol_error(
                    self,
                    raised.exception,
                    ErrorClass.PROPERTY,
                    ErrorCode.VALUE_OUT_OF_RANGE,
                )

    def test_live_server_window_slides_and_resets(self) -> None:
        asyncio.run(self._exercise_window())

    async def _exercise_window(self) -> None:
        server = BACnetServer(
            device_instance=1_092_001,
            device_name="Averaging Window Artifact Test",
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        )
        server.add_averaging(1, "AVG-1", window_interval=60, window_samples=2)
        await server.start()
        try:
            self.assertEqual(
                await self._read(server, PropertyIdentifier.WINDOW_INTERVAL),
                PropertyValue.unsigned(60),
            )
            self.assertEqual(
                await self._read(server, PropertyIdentifier.WINDOW_SAMPLES),
                PropertyValue.unsigned(2),
            )
            await self._assert_empty(server)

            await server.add_averaging_sample_local(AVG, PropertyValue.real(10.0))
            await server.add_averaging_sample_local(AVG, None)
            await server.add_averaging_sample_local(AVG, PropertyValue.real(30.0))
            # The 10 has left the two-slot window; the miss and the 30 remain.
            for property_id, expected in (
                (PropertyIdentifier.MINIMUM_VALUE, PropertyValue.real(30.0)),
                (PropertyIdentifier.MAXIMUM_VALUE, PropertyValue.real(30.0)),
                (PropertyIdentifier.AVERAGE_VALUE, PropertyValue.real(30.0)),
                (PropertyIdentifier.ATTEMPTED_SAMPLES, PropertyValue.unsigned(2)),
                (PropertyIdentifier.VALID_SAMPLES, PropertyValue.unsigned(1)),
            ):
                with self.subTest(property_id=property_id):
                    self.assertEqual(await self._read(server, property_id), expected)

            # A peer's write of Window_Samples empties the window; one past
            # the bound is refused.
            address = await server.local_address()
            async with BACnetClient(
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
                apdu_timeout_ms=2_000,
            ) as client:
                await client.write_property(
                    address,
                    AVG,
                    PropertyIdentifier.WINDOW_SAMPLES,
                    PropertyValue.unsigned(4),
                )
                with self.assertRaises(BacnetProtocolError) as raised:
                    await client.write_property(
                        address,
                        AVG,
                        PropertyIdentifier.WINDOW_SAMPLES,
                        PropertyValue.unsigned(1441),
                    )
                assert_protocol_error(
                    self,
                    raised.exception,
                    ErrorClass.PROPERTY,
                    ErrorCode.VALUE_OUT_OF_RANGE,
                )
            await self._assert_empty(server)
            self.assertEqual(
                await self._read(server, PropertyIdentifier.WINDOW_SAMPLES),
                PropertyValue.unsigned(4),
            )
        finally:
            await server.stop()

    @staticmethod
    async def _read(server: BACnetServer, property_id: object) -> PropertyValue:
        return await server.read_property(AVG, property_id)

    async def _assert_empty(self, server: BACnetServer) -> None:
        """No valid sample: +INF, -INF, NaN and two zero counts."""
        minimum = await self._read(server, PropertyIdentifier.MINIMUM_VALUE)
        maximum = await self._read(server, PropertyIdentifier.MAXIMUM_VALUE)
        average = await self._read(server, PropertyIdentifier.AVERAGE_VALUE)
        self.assertEqual(minimum.value, math.inf)
        self.assertEqual(maximum.value, -math.inf)
        self.assertTrue(math.isnan(average.value), average)
        for property_id in (
            PropertyIdentifier.ATTEMPTED_SAMPLES,
            PropertyIdentifier.VALID_SAMPLES,
        ):
            self.assertEqual(
                await self._read(server, property_id), PropertyValue.unsigned(0)
            )


if __name__ == "__main__":
    unittest.main()
