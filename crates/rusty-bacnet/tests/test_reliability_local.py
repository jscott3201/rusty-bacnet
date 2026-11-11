"""Installed-artifact tests for the application's Reliability route (#2469).

`set_reliability_local` lets the application report a fault it worked out on
one of its own objects, as the Rust server's call does (#2446): Status_Flags'
FAULT follows, a COV subscriber hears of it, and each refusal changes nothing
and sends nothing.
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
    Reliability,
)

P = PropertyIdentifier
AI = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
AV = ObjectIdentifier(ObjectType.ANALOG_VALUE, 2)
CALENDAR = ObjectIdentifier(ObjectType.CALENDAR, 3)
UNKNOWN = ObjectIdentifier(ObjectType.ANALOG_INPUT, 999)
NORMAL_FLAGS = PropertyValue.bit_string(4, b"\x00")
# FAULT, and IN_ALARM with it: the event pass that follows the change puts
# Event_State in FAULT.
FAULT_FLAGS = PropertyValue.bit_string(4, b"\xc0")
NOTIFICATION_TIMEOUT = 2.0
SILENCE_TIMEOUT = 0.25


def installed_stub_method() -> ast.FunctionDef:
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
        if isinstance(node, ast.FunctionDef) and node.name == "set_reliability_local"
    )


class ReliabilityEnumTests(unittest.TestCase):
    def test_constants_and_raw_values(self) -> None:
        self.assertEqual(Reliability.NO_FAULT_DETECTED.to_raw(), 0)
        self.assertEqual(Reliability.NO_SENSOR.to_raw(), 1)
        self.assertEqual(Reliability.MULTI_STATE_OUT_OF_RANGE.to_raw(), 25)
        self.assertEqual(Reliability.NO_SENSOR.name, "NO_SENSOR")
        self.assertEqual(Reliability.from_raw(1), Reliability.NO_SENSOR)
        self.assertIsNone(Reliability.from_raw(64).name)
        self.assertEqual(repr(Reliability.TRIPPED), "Reliability.TRIPPED")


class SetReliabilityLocalTests(unittest.TestCase):
    def test_runtime_and_stub_signature(self) -> None:
        self.assertEqual(
            list(inspect.signature(BACnetServer.set_reliability_local).parameters),
            ["self", "object_id", "reliability"],
        )
        method = installed_stub_method()
        arguments = [*method.args.posonlyargs, *method.args.args]
        self.assertEqual(
            [argument.arg for argument in arguments],
            ["self", "object_id", "reliability"],
        )
        self.assertEqual(
            [ast.unparse(argument.annotation) for argument in arguments[1:]],
            ["ObjectIdentifier", "Reliability"],
        )
        self.assertEqual(ast.unparse(method.returns), "Awaitable[None]")

    def test_a_property_value_is_refused(self) -> None:
        asyncio.run(self._refuse_property_value())

    async def _refuse_property_value(self) -> None:
        server = make_server()
        await server.start()
        try:
            with self.assertRaises(TypeError):
                await server.set_reliability_local(AI, PropertyValue.enumerated(1))  # type: ignore[arg-type]
        finally:
            await server.stop()

    def test_live_server_sets_reports_and_refuses(self) -> None:
        asyncio.run(self._exercise())

    def assert_code(
        self, raised: BacnetProtocolError, cls: ErrorClass, code: ErrorCode
    ) -> None:
        self.assertEqual(raised.error_class, cls.to_raw())
        self.assertEqual(raised.error_code, code.to_raw())

    async def _exercise(self) -> None:
        server = make_server()
        with self.assertRaises(RuntimeError):
            await server.set_reliability_local(AI, Reliability.NO_SENSOR)
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
                    subscriber_process_identifier=1,
                    monitored_object_identifier=AI,
                    confirmed=False,
                    lifetime=60,
                )
                initial = await self._next(notifications)
                self.assertEqual(self._flags(initial), NORMAL_FLAGS)

                # A fault: Reliability and Status_Flags follow, and the
                # subscriber hears of it.
                self.assertIsNone(
                    await server.set_reliability_local(AI, Reliability.NO_SENSOR)
                )
                self.assertEqual(
                    await client.read_property(address, AI, P.RELIABILITY),
                    PropertyValue.enumerated(Reliability.NO_SENSOR.to_raw()),
                )
                self.assertEqual(
                    await client.read_property(address, AI, P.STATUS_FLAGS),
                    FAULT_FLAGS,
                )
                self.assertEqual(self._flags(await self._next(notifications)), FAULT_FLAGS)

                # Clearing it is reported too.
                await server.set_reliability_local(AI, Reliability.NO_FAULT_DETECTED)
                self.assertEqual(
                    await server.read_property(AI, P.STATUS_FLAGS), NORMAL_FLAGS
                )
                self.assertEqual(self._flags(await self._next(notifications)), NORMAL_FLAGS)

                # Values outside BACnetReliability.
                for raw in (11, 26, 63, 65_536):
                    with self.subTest(raw=raw):
                        await self._refused(
                            server,
                            AI,
                            Reliability.from_raw(raw),
                            ErrorClass.PROPERTY,
                            ErrorCode.VALUE_OUT_OF_RANGE,
                        )
                await self._silence(notifications)
                # A vendor value is taken (on an object nobody subscribes to).
                await server.set_reliability_local(AV, Reliability.from_raw(64))
                self.assertEqual(
                    await server.read_property(AV, P.RELIABILITY),
                    PropertyValue.enumerated(64),
                )
                self.assertIn(
                    "FAULT", (await server.read_property(AV, P.STATUS_FLAGS)).value.names
                )

                # While evaluation is inhibited.
                await server.write_property_local(
                    AI,
                    P.RELIABILITY_EVALUATION_INHIBIT,
                    PropertyValue.boolean(True),
                    source_object=None,
                )
                await self._refused(
                    server,
                    AI,
                    Reliability.OVER_RANGE,
                    ErrorClass.PROPERTY,
                    ErrorCode.WRITE_ACCESS_DENIED,
                )
                await self._silence(notifications)
                await server.write_property_local(
                    AI,
                    P.RELIABILITY_EVALUATION_INHIBIT,
                    PropertyValue.boolean(False),
                    source_object=None,
                )

                # Out of service, where a client may be simulating a fault.
                await server.write_property_local(
                    AI, P.OUT_OF_SERVICE, PropertyValue.boolean(True), source_object=None
                )
                self.assertEqual(
                    self._flags(await self._next(notifications)),
                    PropertyValue.bit_string(4, b"\x10"),
                )
                await self._refused(
                    server,
                    AI,
                    Reliability.NO_SENSOR,
                    ErrorClass.PROPERTY,
                    ErrorCode.WRITE_ACCESS_DENIED,
                )

                # An object that isn't there, and one that takes no Reliability.
                with self.assertRaises(BacnetProtocolError) as raised:
                    await server.set_reliability_local(UNKNOWN, Reliability.NO_SENSOR)
                self.assert_code(raised.exception, ErrorClass.OBJECT, ErrorCode.UNKNOWN_OBJECT)
                with self.assertRaises(BacnetProtocolError) as raised:
                    await server.set_reliability_local(CALENDAR, Reliability.NO_SENSOR)
                self.assert_code(
                    raised.exception,
                    ErrorClass.OBJECT,
                    ErrorCode.OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
                )
                await self._silence(notifications)
        finally:
            if listener is not None:
                listener.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await listener
            await server.stop()

    async def _refused(
        self,
        server: BACnetServer,
        oid: ObjectIdentifier,
        reliability: Reliability,
        error_class: ErrorClass,
        error_code: ErrorCode,
    ) -> None:
        """Refused with the given error, changing nothing."""
        before = await server.read_property(oid, P.RELIABILITY)
        with self.assertRaises(BacnetProtocolError) as raised:
            await server.set_reliability_local(oid, reliability)
        self.assert_code(raised.exception, error_class, error_code)
        self.assertEqual(await server.read_property(oid, P.RELIABILITY), before)

    @staticmethod
    async def _next(notifications: asyncio.Queue[Any]) -> Any:
        notification = await asyncio.wait_for(
            notifications.get(), timeout=NOTIFICATION_TIMEOUT
        )
        assert notification.monitored_object_identifier == AI
        return notification

    @staticmethod
    def _flags(notification: Any) -> PropertyValue:
        (flags,) = [
            item["value"]
            for item in notification.values
            if item["property_id"] == P.STATUS_FLAGS
        ]
        return flags

    async def _silence(self, notifications: asyncio.Queue[Any]) -> None:
        with self.assertRaises(asyncio.TimeoutError):
            await asyncio.wait_for(notifications.get(), timeout=SILENCE_TIMEOUT)


def make_server() -> BACnetServer:
    server = BACnetServer(
        device_instance=2_469_001,
        device_name="Reliability Local Test",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )
    server.add_analog_input(1, "AI-1", present_value=20.0)
    server.add_analog_value(2, "AV-2")
    server.add_calendar(3, "CAL-3")
    return server


if __name__ == "__main__":
    unittest.main()
