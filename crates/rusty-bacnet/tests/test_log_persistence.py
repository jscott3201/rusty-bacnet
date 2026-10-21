"""Logs kept across a restart with ``storage_path`` (#1541).

``add_trend_log``, ``add_trend_log_multiple`` and ``add_event_log`` take a
keyword-only ``storage_path``. The log commits to ``<path>.slot0`` and
``.slot1`` on a cadence and once more at ``stop()``; a server started on the
same files serves the saved records, numbering and settings, plus one
LOG_INTERRUPTED record that marks the restart.
"""

from __future__ import annotations

import ast
import asyncio
import inspect
import tempfile
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetError,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

WAIT = 5.0
TREND_LOG = ObjectIdentifier(ObjectType.TREND_LOG, 1)
EVENT_LOG = ObjectIdentifier(ObjectType.EVENT_LOG, 1)
MULTIPLE = ObjectIdentifier(ObjectType.TREND_LOG_MULTIPLE, 1)
METHODS = ("add_trend_log", "add_trend_log_multiple", "add_event_log")

# A log-status datum carrying LOG_INTERRUPTED alone: the [1] field opened,
# its [0] log-status choice as a three-bit BIT STRING (five unused bits,
# log-interrupted is bit 2), and the [1] field closed. All three log record
# productions put it right after the timestamp.
INTERRUPTED = bytes([0x1E, 0x0A, 0x05, 0x20, 0x1F])


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


class StoragePathSignatureTests(unittest.TestCase):
    def test_each_log_takes_a_keyword_only_storage_path(self) -> None:
        for name in METHODS:
            with self.subTest(name=name):
                parameter = inspect.signature(getattr(BACnetServer, name)).parameters[
                    "storage_path"
                ]
                self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
                self.assertIsNone(parameter.default)
                stub = installed_stub_method(name)
                self.assertIn(
                    "storage_path", [argument.arg for argument in stub.args.kwonlyargs]
                )

    def test_an_empty_or_unreadable_storage_path_is_refused(self) -> None:
        server = BACnetServer(1_541_000, interface="127.0.0.1", port=0)
        for name in METHODS:
            with self.subTest(name=name):
                with self.assertRaisesRegex(BacnetError, "must not be empty"):
                    getattr(server, name)(1, "Log", storage_path="")
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory) / "log"
            Path(f"{base}.slot0").write_bytes(b"not a log snapshot")
            with self.assertRaises(BacnetError):
                server.add_trend_log(1, "TL-1", storage_path=str(base))
            for name in METHODS:
                with self.subTest(path_object=name):
                    with self.assertRaises(TypeError):
                        getattr(server, name)(2, "Log", storage_path=Path(directory) / "p")
            # Past what a slot holds for a Trend Log Multiple.
            with self.assertRaisesRegex(BacnetError, "at most 15849"):
                server.add_trend_log_multiple(
                    3, "TLM-3", 15_850, storage_path=str(Path(directory) / "big")
                )
        self.assertEqual(server._pending_registration_count(), 0)


class LogRestartTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.state = Path(directory.name)
        self.client = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
        await self.client.__aenter__()

        async def stop_client() -> None:
            await self.client.__aexit__(None, None, None)

        self.addAsyncCleanup(stop_client)

    async def start(self, buffer_size: int, log_interval: int) -> BACnetServer:
        server = BACnetServer(1_541_001, interface="127.0.0.1", port=0)
        server.add_analog_input(1, "AI-1", present_value=21.5)
        server.add_trend_log_multiple(
            1,
            "TLM-1",
            buffer_size,
            members=[
                (
                    ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
                    PropertyIdentifier.PRESENT_VALUE,
                )
            ],
            logging_type="triggered",
            storage_path=str(self.state / "tlm-1"),
        )
        # No reference, so nothing is polled; the interval is only kept.
        server.add_trend_log(
            1,
            "TL-1",
            log_interval=log_interval,
            storage_path=str(self.state / "tl-1"),
        )
        server.add_event_log(1, "EL-1", buffer_size, storage_path=str(self.state / "el-1"))
        await server.start()
        return server

    async def read(self, server: BACnetServer, oid: ObjectIdentifier, prop) -> PropertyValue:
        return await server.read_property(oid, prop)

    async def records(self, server: BACnetServer, oid: ObjectIdentifier) -> dict:
        address = await server.local_address()
        return await self.client.read_range(
            address,
            oid,
            PropertyIdentifier.LOG_BUFFER,
            range_type="sequence",
            reference_seq=1,
            count=50,
        )

    async def test_a_restarted_server_serves_saved_logs_plus_one_interruption(self) -> None:
        server = await self.start(buffer_size=20, log_interval=500)
        try:
            for expected in (1, 2, 3):
                await server.write_property_local(
                    MULTIPLE,
                    PropertyIdentifier.TRIGGER,
                    PropertyValue.boolean(True),
                    source_object=None,
                )
                async with asyncio.timeout(WAIT):
                    while await self.read(
                        server, MULTIPLE, PropertyIdentifier.TOTAL_RECORD_COUNT
                    ) != PropertyValue.unsigned(expected):
                        await asyncio.sleep(0.02)
            # A peer's write of a saved setting.
            await server.write_property_local(
                TREND_LOG,
                PropertyIdentifier.LOG_INTERVAL,
                PropertyValue.unsigned(12_345),
                source_object=None,
            )
            before = await self.records(server, MULTIPLE)
            self.assertEqual(before["item_count"], 3)
        finally:
            # stop() commits each log and waits for the commit.
            await server.stop()

        # Another Log_Interval keyword: the saved one wins. Buffer_Size is the
        # application's, so the new 50 wins over the saved 20.
        server = await self.start(buffer_size=50, log_interval=500)
        try:
            after = await self.records(server, MULTIPLE)
            self.assertEqual(after["item_count"], 4)
            self.assertEqual(after["first_sequence_number"], 1)
            self.assertTrue(after["item_data"].startswith(before["item_data"]))
            self.assertIn(INTERRUPTED, after["item_data"][len(before["item_data"]) :])
            self.assertEqual(
                await self.read(server, MULTIPLE, PropertyIdentifier.TOTAL_RECORD_COUNT),
                PropertyValue.unsigned(4),
            )
            self.assertEqual(
                await self.read(server, MULTIPLE, PropertyIdentifier.BUFFER_SIZE),
                PropertyValue.unsigned(50),
            )
            self.assertEqual(
                await self.read(server, TREND_LOG, PropertyIdentifier.LOG_INTERVAL),
                PropertyValue.unsigned(12_345),
            )
            for oid in (TREND_LOG, EVENT_LOG):
                with self.subTest(oid=oid):
                    page = await self.records(server, oid)
                    self.assertEqual(page["item_count"], 1)
                    self.assertIn(INTERRUPTED, page["item_data"])
                    self.assertEqual(
                        await self.read(server, oid, PropertyIdentifier.TOTAL_RECORD_COUNT),
                        PropertyValue.unsigned(1),
                    )
        finally:
            await server.stop()
        for name in ("tlm-1", "tl-1", "el-1"):
            self.assertTrue((self.state / f"{name}.slot0").exists() or (self.state / f"{name}.slot1").exists())


if __name__ == "__main__":
    unittest.main()
