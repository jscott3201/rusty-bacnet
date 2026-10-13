"""Installed-artifact tests for an Event Log that collects the event
notifications the server receives (#1346) and reports BUFFER_READY (#1347).

`add_event_log(..., log_received_notifications=True)` opts a log in; a raw
UnconfirmedEventNotification sent to the running server then shows up in
that log's buffer, read back with ReadRange, and in no other log.
"""

from __future__ import annotations

import ast
import asyncio
import inspect
import socket
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

COLLECTING = ObjectIdentifier(ObjectType.EVENT_LOG, 1)
PLAIN = ObjectIdentifier(ObjectType.EVENT_LOG, 2)
WAIT = 5.0

# Device 50's OUT_OF_RANGE alarm about its Analog Input 3: process 7,
# sequence-number timestamp 9, class 4, priority 100, ALARM, no ack, NORMAL
# to HIGH_LIMIT, with no event values.
NOTIFICATION = bytes.fromhex(
    "09 07 1c 02000032 2c 00000003 3e 19 09 3f 49 04 59 64 69 05"
    " 89 00 99 00 a9 00 b9 03"
)
# B/IP Original-Unicast-NPDU, NPDU 01 00, UnconfirmedEventNotification (3).
APDU = bytes.fromhex("01 00 10 03") + NOTIFICATION
FRAME = bytes.fromhex("81 0a") + (4 + len(APDU)).to_bytes(2, "big") + APDU


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


class EventLogStubContractTests(unittest.TestCase):
    def test_runtime_and_stub_take_the_same_arguments(self) -> None:
        parameters = inspect.signature(BACnetServer.add_event_log).parameters
        positional = ["self", "instance", "name", "buffer_size", "log_received_notifications"]
        self.assertEqual(list(parameters), [*positional, "total_record_count"])
        self.assertIs(parameters["log_received_notifications"].default, False)
        self.assertEqual(
            parameters["total_record_count"].kind, inspect.Parameter.KEYWORD_ONLY
        )
        self.assertEqual(parameters["total_record_count"].default, 0)
        method = installed_stub_method("add_event_log")
        self.assertEqual([argument.arg for argument in method.args.args], positional)
        self.assertEqual(
            [argument.arg for argument in method.args.kwonlyargs], ["total_record_count"]
        )
        docs = ast.get_docstring(method)
        assert docs is not None
        for phrase in ("received_not_logged", "Notification_Threshold", "BUFFER_READY"):
            with self.subTest(documented=phrase):
                self.assertIn(phrase, docs)


class EventLogLiveServerTests(unittest.TestCase):
    def test_only_the_collecting_log_records_a_received_notification(self) -> None:
        asyncio.run(self._exercise())

    async def _exercise(self) -> None:
        server = BACnetServer(
            1_346_000,
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        )
        server.add_event_log(1, "EL-1", 16, log_received_notifications=True)
        server.add_event_log(2, "EL-2", 16)
        await server.start()
        loop = asyncio.get_running_loop()
        try:
            ip, port = (await server.local_address()).split(":")
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
                peer.bind(("127.0.0.1", 0))
                peer.setblocking(False)
                await loop.sock_sendto(peer, FRAME, (ip, int(port)))
            address = await server.local_address()
            async with BACnetClient(
                interface="127.0.0.1", port=0, apdu_timeout_ms=2000
            ) as client:
                async with asyncio.timeout(WAIT):
                    while True:
                        result = await client.read_range(
                            address, COLLECTING, PropertyIdentifier.LOG_BUFFER
                        )
                        if result["item_count"] >= 1:
                            break
                        await asyncio.sleep(0.05)
                self.assertEqual(result["item_count"], 1)
                # The record re-encodes the decoded notification, inside
                # the notification choice [1] of the record's datum [1].
                self.assertIn(NOTIFICATION, result["item_data"])
                plain = await client.read_range(
                    address, PLAIN, PropertyIdentifier.LOG_BUFFER
                )
                self.assertEqual(plain["item_count"], 0)
            # BUFFER_READY is configured through the log's own rows.
            self.assertEqual(
                await server.read_property(
                    COLLECTING, PropertyIdentifier.NOTIFICATION_THRESHOLD
                ),
                PropertyValue.unsigned(0),
            )
            counters = await server.event_notification_counters()
            self.assertEqual(counters["received_not_logged"], 0)
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
