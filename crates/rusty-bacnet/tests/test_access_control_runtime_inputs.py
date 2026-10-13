"""Installed-artifact tests for the access-control runtime inputs (#1132).

report_access_event_local, report_credential_read_local and
report_door_state_local hand an Access Point's access event, a Credential
Data Input's read and an Access Door's hardware state to a running server
as one local write each, so COV reports follow. While Out_Of_Service is
TRUE the point refuses an event, and the reader and the door keep a report
aside for the return to service.
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
    BACnetTimeStamp,
    BacnetProtocolError,
    ErrorClass,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

P = PropertyIdentifier
NOTIFICATION_TIMEOUT = 2.0
POINT = ObjectIdentifier(ObjectType.ACCESS_POINT, 1)
READER = ObjectIdentifier(ObjectType.CREDENTIAL_DATA_INPUT, 1)
DOOR = ObjectIdentifier(ObjectType.ACCESS_DOOR, 1)
BADGE = ObjectIdentifier(ObjectType.ACCESS_CREDENTIAL, 5)
GRANTED = 1
WIEGAND26 = 8
CARD = (WIEGAND26, 0, b"\x12\x34\x56")
# CARD as Clause 21 puts it: format type [0], class [1], value [2].
CARD_OCTETS = bytes([0x09, WIEGAND26, 0x19, 0x00, 0x2B, 0x12, 0x34, 0x56])
OPENED = 1
FORCED_OPEN = 3
TAMPER = 4
OFFNORMAL = 2

ROUTES = (
    ("report_access_event_local", ["self", "object_id", "event", "tag"],
     ["time", "credential", "authentication_factor"]),
    ("report_credential_read_local", ["self", "object_id", "factor"], ["update_time"]),
    ("report_door_state_local", ["self", "object_id"],
     ["door_status", "lock_status", "door_alarm_state"]),
)


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
        device_instance=1_132_001,
        device_name="Access Control Runtime Inputs Test",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )
    server.add_access_point(1, "Lobby")
    server.add_credential_data_input(1, "Reader", supported_formats=[(WIEGAND26, 0)])
    server.add_access_door(1, "Main Entry", alarm_values=[FORCED_OPEN])
    return server


class RuntimeInputStubContractTests(unittest.TestCase):
    def test_runtime_and_stub_expose_the_routes(self) -> None:
        for name, positional, keywords in ROUTES:
            with self.subTest(route=name):
                parameters = inspect.signature(getattr(BACnetServer, name)).parameters
                self.assertEqual(list(parameters), positional + keywords)
                for keyword in keywords:
                    self.assertIs(parameters[keyword].kind, inspect.Parameter.KEYWORD_ONLY)
                    self.assertIsNone(parameters[keyword].default)
                method = installed_stub_method(name)
                self.assertEqual([a.arg for a in method.args.args], positional)
                self.assertEqual([a.arg for a in method.args.kwonlyargs], keywords)
                self.assertEqual(ast.unparse(method.returns), "Awaitable[None]")
                docs = ast.get_docstring(method)
                assert docs is not None
                docs = " ".join(docs.split())
                phrases = ["out of service", "OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED"]
                if name == "report_access_event_local":
                    phrases.append("WRITE_ACCESS_DENIED")
                for phrase in phrases:
                    self.assertIn(phrase, docs)


class RuntimeInputLiveServerTests(unittest.TestCase):
    def assert_error(self, raised: BacnetProtocolError, error_class: ErrorClass,
                     code: ErrorCode) -> None:
        self.assertEqual(raised.error_class, error_class.to_raw())
        self.assertEqual(raised.error_code, code.to_raw())

    def test_routes_reach_the_objects_and_refuse_out_of_service(self) -> None:
        asyncio.run(self._routes())

    async def _routes(self) -> None:
        server = make_server()
        with self.assertRaises(RuntimeError):
            await server.report_door_state_local(DOOR, door_status=OPENED)
        await server.start()
        try:
            async def read(oid: ObjectIdentifier, prop: PropertyIdentifier) -> PropertyValue:
                return await server.read_property(oid, prop)

            self.assertIsNone(await server.report_access_event_local(
                POINT, GRANTED, 7, time=BACnetTimeStamp.sequence_number(9),
                credential=BADGE, authentication_factor=CARD,
            ))
            self.assertEqual((await read(POINT, P.ACCESS_EVENT)).value, GRANTED)
            self.assertEqual((await read(POINT, P.ACCESS_EVENT_TAG)).value, 7)
            self.assertEqual(await read(POINT, P.ACCESS_EVENT_AUTHENTICATION_FACTOR),
                             PropertyValue.application_data(CARD_OCTETS))

            await server.report_credential_read_local(
                READER, CARD, update_time=BACnetTimeStamp.sequence_number(5))
            self.assertEqual(await read(READER, P.PRESENT_VALUE),
                             PropertyValue.application_data(CARD_OCTETS))

            await server.report_door_state_local(
                DOOR, door_status=OPENED, door_alarm_state=FORCED_OPEN)
            self.assertEqual((await read(DOOR, P.DOOR_STATUS)).value, OPENED)
            self.assertEqual((await read(DOOR, P.DOOR_ALARM_STATE)).value, FORCED_OPEN)
            # The event pass ran with the write: the door is in alarm already.
            self.assertEqual((await read(DOOR, P.EVENT_STATE)).value, OFFNORMAL)

            # Values the objects refuse.
            for route in (
                lambda: server.report_access_event_local(POINT, GRANTED, 8, credential=DOOR),
                # Reserved: neither named nor in the proprietary 512..65535.
                lambda: server.report_access_event_local(POINT, 200, 8),
                lambda: server.report_credential_read_local(READER, (9, 0, b"\x01")),
                lambda: server.report_door_state_local(DOOR, door_alarm_state=TAMPER),
            ):
                with self.assertRaises(BacnetProtocolError) as raised:
                    await route()
                self.assert_error(raised.exception, ErrorClass.PROPERTY,
                                  ErrorCode.VALUE_OUT_OF_RANGE)
            # The wrong object, and no object.
            for route, code in (
                (lambda: server.report_door_state_local(POINT, door_status=OPENED),
                 ErrorCode.OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED),
                (lambda: server.report_credential_read_local(DOOR, CARD),
                 ErrorCode.OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED),
                (lambda: server.report_access_event_local(
                    ObjectIdentifier(ObjectType.ACCESS_POINT, 9), GRANTED, 1),
                 ErrorCode.UNKNOWN_OBJECT),
            ):
                with self.assertRaises(BacnetProtocolError) as raised:
                    await route()
                self.assert_error(raised.exception, ErrorClass.OBJECT, code)

            # Out of service the point refuses an event, and nothing changes.
            async def out_of_service(oid: ObjectIdentifier, value: bool) -> None:
                await server.write_property_local(
                    oid, P.OUT_OF_SERVICE, PropertyValue.boolean(value), source_object=None)

            for oid in (POINT, READER, DOOR):
                await out_of_service(oid, True)
            with self.assertRaises(BacnetProtocolError) as raised:
                await server.report_access_event_local(POINT, GRANTED, 9)
            self.assert_error(raised.exception, ErrorClass.PROPERTY,
                              ErrorCode.WRITE_ACCESS_DENIED)
            # The entry edge's own OUT_OF_SERVICE (10) event stays served.
            self.assertEqual((await read(POINT, P.ACCESS_EVENT)).value, 10)

            # The reader and the door keep a report aside: the served values
            # stay until the return to service, which serves the latest ones.
            other_card = (WIEGAND26, 0, b"\x65\x43\x21")
            other_octets = CARD_OCTETS[:5] + b"\x65\x43\x21"
            await server.report_credential_read_local(READER, other_card)
            await server.report_door_state_local(DOOR, door_status=0)
            await server.report_door_state_local(DOOR, door_alarm_state=0)
            self.assertEqual(await read(READER, P.PRESENT_VALUE),
                             PropertyValue.application_data(CARD_OCTETS))
            self.assertEqual((await read(DOOR, P.DOOR_STATUS)).value, OPENED)
            self.assertEqual((await read(DOOR, P.DOOR_ALARM_STATE)).value, FORCED_OPEN)
            for oid in (READER, DOOR):
                await out_of_service(oid, False)
            self.assertEqual(await read(READER, P.PRESENT_VALUE),
                             PropertyValue.application_data(other_octets))
            self.assertEqual((await read(DOOR, P.DOOR_STATUS)).value, 0)
            self.assertEqual((await read(DOOR, P.DOOR_ALARM_STATE)).value, 0)
        finally:
            await server.stop()

    def test_a_read_sends_the_readers_cov_report(self) -> None:
        asyncio.run(self._cov())

    async def _cov(self) -> None:
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
                    subscriber_process_identifier=1132,
                    monitored_object_identifier=READER,
                    confirmed=False,
                    lifetime=60,
                )
                await asyncio.wait_for(notifications.get(), timeout=NOTIFICATION_TIMEOUT)
                await server.report_credential_read_local(
                    READER, CARD, update_time=BACnetTimeStamp.sequence_number(6))
                notification = await asyncio.wait_for(
                    notifications.get(), timeout=NOTIFICATION_TIMEOUT)
                self.assertEqual(notification.monitored_object_identifier, READER)
                self.assertIn(
                    PropertyValue.application_data(CARD_OCTETS),
                    [item["value"] for item in notification.values
                     if item["property_id"] == P.PRESENT_VALUE],
                )
        finally:
            if listener is not None:
                listener.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await listener
            await server.stop()


if __name__ == "__main__":
    unittest.main()
