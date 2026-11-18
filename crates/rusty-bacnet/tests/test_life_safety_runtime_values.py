"""Installed-artifact tests for the Life Safety runtime value routes (#1123).

`set_present_value_local` now takes a running Life Safety Point's or Zone's
Present_Value, and `set_tracking_value_local` its Tracking_Value. Both notify
through the server's Life Safety COV path; while Out_Of_Service is set an
application Tracking_Value waits for the return to service.
"""

from __future__ import annotations

import ast
import asyncio
import contextlib
import inspect
import unittest
from pathlib import Path
from typing import Any

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
NOTIFICATION_TIMEOUT = 2.0
SILENCE_TIMEOUT = 0.25
POINT = ObjectIdentifier(ObjectType.LIFE_SAFETY_POINT, 1)
ZONE = ObjectIdentifier(ObjectType.LIFE_SAFETY_ZONE, 1)
AV = ObjectIdentifier(ObjectType.ANALOG_VALUE, 2)
PV = PropertyIdentifier.PRESENT_VALUE
TV = PropertyIdentifier.TRACKING_VALUE
QUIET = PropertyValue.enumerated(0)
PRE_ALARM = PropertyValue.enumerated(1)
ALARM = PropertyValue.enumerated(2)


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
        device_instance=1_123_001,
        device_name="Life Safety Runtime Values Artifact Test",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )
    server.add_life_safety_point(1, "LSP-1")
    server.add_life_safety_zone(1, "LSZ-1")
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


class LifeSafetyStubContractTests(unittest.TestCase):
    def test_runtime_and_stub_expose_the_tracking_value_route(self) -> None:
        runtime = list(
            inspect.signature(BACnetServer.set_tracking_value_local).parameters
        )
        self.assertEqual(runtime, ["self", "object_id", "value"])
        method = installed_stub_method("set_tracking_value_local")
        self.assertEqual(
            [argument.arg for argument in method.args.args],
            ["self", "object_id", "value"],
        )
        self.assertEqual(ast.unparse(method.returns), "Awaitable[None]")
        docs = ast.get_docstring(method)
        assert docs is not None
        for phrase in (
            "BACnetLifeSafetyState",
            "VALUE_OUT_OF_RANGE",
            "OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED",
            "Out_Of_Service",
            "SubscribeCOVProperty",
        ):
            with self.subTest(documented=phrase):
                self.assertIn(phrase, docs)
        present_value_docs = ast.get_docstring(
            installed_stub_method("set_present_value_local")
        )
        assert present_value_docs is not None
        self.assertIn("Life Safety Point or Zone", present_value_docs)


class LifeSafetyLiveServerTests(unittest.TestCase):
    def test_live_server_takes_runtime_values_and_refuses_bad_ones(self) -> None:
        asyncio.run(self._exercise_values())

    async def _exercise_values(self) -> None:
        server = make_server()
        with self.assertRaises(RuntimeError):
            await server.set_tracking_value_local(POINT, ALARM)

        await server.start()
        try:
            for oid in (POINT, ZONE):
                with self.subTest(object=oid):
                    self.assertIsNone(await server.set_present_value_local(oid, ALARM))
                    self.assertIsNone(
                        await server.set_tracking_value_local(oid, PRE_ALARM)
                    )
                    self.assertEqual(await server.read_property(oid, PV), ALARM)
                    self.assertEqual(await server.read_property(oid, TV), PRE_ALARM)

            for value, code in (
                (PropertyValue.enumerated(35), ErrorCode.VALUE_OUT_OF_RANGE),
                (PropertyValue.enumerated(65_536), ErrorCode.VALUE_OUT_OF_RANGE),
                (PropertyValue.unsigned(2), ErrorCode.INVALID_DATA_TYPE),
            ):
                for route in (
                    server.set_present_value_local,
                    server.set_tracking_value_local,
                ):
                    with self.subTest(refused=value, route=route.__name__):
                        with self.assertRaises(BacnetProtocolError) as raised:
                            await route(POINT, value)
                        assert_protocol_error(
                            self, raised.exception, ErrorClass.PROPERTY, code
                        )
            self.assertEqual(await server.read_property(POINT, PV), ALARM)
            self.assertEqual(await server.read_property(POINT, TV), PRE_ALARM)

            for target, code in (
                (AV, ErrorCode.OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED),
                (
                    ObjectIdentifier(ObjectType.LIFE_SAFETY_POINT, 9),
                    ErrorCode.UNKNOWN_OBJECT,
                ),
            ):
                with self.subTest(target=target):
                    with self.assertRaises(BacnetProtocolError) as raised:
                        await server.set_tracking_value_local(target, ALARM)
                    assert_protocol_error(
                        self, raised.exception, ErrorClass.OBJECT, code
                    )

            # Out of service the application's Tracking_Value is held for the
            # return to service; Present_Value is served at once.
            await server.write_property_local(
                ZONE,
                PropertyIdentifier.OUT_OF_SERVICE,
                PropertyValue.boolean(True),
                source_object=None,
            )
            await server.set_tracking_value_local(ZONE, ALARM)
            await server.set_present_value_local(ZONE, QUIET)
            self.assertEqual(await server.read_property(ZONE, TV), PRE_ALARM)
            self.assertEqual(await server.read_property(ZONE, PV), QUIET)
            await server.write_property_local(
                ZONE,
                PropertyIdentifier.OUT_OF_SERVICE,
                PropertyValue.boolean(False),
                source_object=None,
            )
            self.assertEqual(await server.read_property(ZONE, TV), ALARM)
        finally:
            await server.stop()

    def test_live_server_notifies_both_routes(self) -> None:
        asyncio.run(self._exercise_cov())

    async def _exercise_cov(self) -> None:
        server = make_server()
        notifications: asyncio.Queue[Any] = asyncio.Queue()
        listener: asyncio.Task[None] | None = None
        await server.start()
        try:
            address = await server.local_address()
            async with BACnetClient(
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
                apdu_timeout_ms=2_000,
            ) as client:
                iterator = await client.cov_notifications()

                async def collect() -> None:
                    async for notification in iterator:
                        notifications.put_nowait(notification)

                listener = asyncio.create_task(collect())
                await client.subscribe_cov(
                    address,
                    subscriber_process_identifier=1123,
                    monitored_object_identifier=POINT,
                    confirmed=False,
                    lifetime=60,
                )
                await asyncio.wait_for(notifications.get(), timeout=NOTIFICATION_TIMEOUT)

                await server.set_present_value_local(POINT, ALARM)
                notification = await asyncio.wait_for(
                    notifications.get(), timeout=NOTIFICATION_TIMEOUT
                )
                self.assertEqual(notification.monitored_object_identifier, POINT)
                self.assertIn(
                    ALARM,
                    [
                        item["value"]
                        for item in notification.values
                        if item["property_id"] == PV
                    ],
                )

                # The Tracking_Value reports come as COV-multiple
                # notifications (#2654), and the server's counters show them.
                sent = (await server.cov_counters())["notifications_sent"]
                await client.subscribe_cov_property_multiple(
                    address,
                    1124,
                    [(POINT, [(TV, None, None, False)])],
                    False,
                    max_notification_delay=10,
                    lifetime=60,
                )
                sent = await self._wait_for_sent(server, sent + 1)  # initial
                initial = await asyncio.wait_for(
                    notifications.get(), timeout=NOTIFICATION_TIMEOUT
                )
                self.assertEqual(
                    (initial.kind, initial.subscriber_process_identifier),
                    ("multiple", 1124),
                )
                await server.set_tracking_value_local(POINT, PRE_ALARM)
                sent = await self._wait_for_sent(server, sent + 1)
                changed = await asyncio.wait_for(
                    notifications.get(), timeout=NOTIFICATION_TIMEOUT
                )
                self.assertEqual(changed.kind, "multiple")
                [reported] = changed.objects
                self.assertEqual(reported["monitored_object_identifier"], POINT)
                self.assertIn(
                    PRE_ALARM,
                    [item["value"] for item in reported["values"] if item["property_id"] == TV],
                )
                # SubscribeCOV doesn't carry Tracking_Value: no ordinary report.
                with self.assertRaises(asyncio.TimeoutError):
                    await asyncio.wait_for(notifications.get(), timeout=SILENCE_TIMEOUT)
                self.assertEqual(
                    (await server.cov_counters())["notifications_sent"], sent
                )
        finally:
            if listener is not None:
                listener.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await listener
            await server.stop()

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


if __name__ == "__main__":
    unittest.main()
