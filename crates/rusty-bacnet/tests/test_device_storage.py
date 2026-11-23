"""The Device's storage (#2730): ``device_storage_path`` on ``BACnetServer``
and the endpoints.

The Device keeps what a peer writes to it in that file: a written
Description and, with ``clock_zone_writable``, a written UTC_Offset. A
server started again on the same path serves both; with the opt-in off the
kept offset stays in the file, unapplied. Without a path such a write lasts
until ``stop()``. A file that can't be loaded raises BacnetError at
``start()``, as an object's ``storage_path`` does, naming the path (and,
for another Device's file, both Devices), and leaves the registrations for
the next start; a write whose save fails is refused with DEVICE /
OPERATIONAL_PROBLEM. A relative path is resolved at construction.
"""

from __future__ import annotations

import ast
import importlib.util
import inspect
import os
import tempfile
import unittest
from pathlib import Path

from rusty_bacnet import (
    BACnetClient,
    BacnetError,
    BacnetProtocolError,
    BACnetServer,
    BipEndpoint,
    ErrorClass,
    ErrorCode,
    MstpEndpoint,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
    ScEndpoint,
)

INSTANCE = 2730
DEVICE = ObjectIdentifier(ObjectType.DEVICE, INSTANCE)
P = PropertyIdentifier
LOBBY = PropertyValue.character_string("Lobby")
EMPTY = PropertyValue.character_string("")


class DeviceStorageTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        # Two directories down, neither made: the first save makes them.
        self.path = Path(directory.name) / "state" / "device.tags"

    def server(self, stored: bool = True, writable: bool = True) -> BACnetServer:
        return BACnetServer(
            INSTANCE, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
            utc_offset_minutes=300, clock_zone_writable=writable,
            device_storage_path=str(self.path) if stored else None,
        )

    async def started(self, stored: bool = True, writable: bool = True) -> BACnetServer:
        server = self.server(stored, writable)
        await server.start()
        return server

    async def peer_writes(self, server: BACnetServer, *writes) -> None:
        """Write each ``(property, value)`` to the Device as a peer does."""
        address = await server.local_address()
        async with BACnetClient(interface="127.0.0.1", port=0) as client:
            for prop, value in writes:
                await client.write_property(address, DEVICE, prop, value)

    async def read(self, server: BACnetServer, prop) -> PropertyValue:
        return await server.read_property(DEVICE, prop)

    async def test_a_server_started_again_on_the_path_serves_what_peers_wrote(self) -> None:
        server = await self.started()
        try:
            self.assertFalse(self.path.exists())
            await self.peer_writes(server, (P.DESCRIPTION, LOBBY),
                                   (P.UTC_OFFSET, PropertyValue.signed(-60)))
            self.assertEqual(await self.read(server, P.DESCRIPTION), LOBBY)
        finally:
            await server.stop()
        self.assertTrue(self.path.exists())

        server = await self.started()
        try:
            self.assertEqual(await self.read(server, P.DESCRIPTION), LOBBY)
            self.assertEqual((await self.read(server, P.UTC_OFFSET)).value, -60)
        finally:
            await server.stop()

        # The opt-in off: the kept offset stays in the file, unapplied.
        before = self.path.read_bytes()
        server = await self.started(writable=False)
        try:
            self.assertEqual(await self.read(server, P.DESCRIPTION), LOBBY)
            self.assertEqual((await self.read(server, P.UTC_OFFSET)).value, 300)
        finally:
            await server.stop()
        self.assertEqual(self.path.read_bytes(), before)

    async def test_without_a_path_a_written_description_lasts_until_stop(self) -> None:
        server = await self.started(stored=False)
        try:
            await self.peer_writes(server, (P.DESCRIPTION, LOBBY))
            self.assertEqual(await self.read(server, P.DESCRIPTION), LOBBY)
        finally:
            await server.stop()
        server = await self.started(stored=False)
        try:
            self.assertEqual(await self.read(server, P.DESCRIPTION), EMPTY)
        finally:
            await server.stop()
        self.assertFalse(self.path.exists())

    async def test_a_file_that_does_not_load_raises_at_start(self) -> None:
        self.path.parent.mkdir(parents=True)
        self.path.write_bytes(b"not a Tags file")
        server = self.server()
        server.add_analog_input(1, "AI-1")
        with self.assertRaises(BacnetError) as raised:
            await server.start()
        self.assertIn(str(self.path), str(raised.exception))
        self.assertIn("no valid header", str(raised.exception))
        self.assertEqual(self.path.read_bytes(), b"not a Tags file")
        # Raised before the registrations were taken: a repaired file starts.
        self.assertEqual(server._pending_registration_count(), 1)
        self.path.unlink()
        await server.start()
        await server.stop()

    async def test_another_devices_file_names_both_and_the_way_on(self) -> None:
        server = await self.started()
        try:
            await self.peer_writes(server, (P.DESCRIPTION, LOBBY))
        finally:
            await server.stop()
        renumbered = BACnetServer(INSTANCE + 1, interface="127.0.0.1", port=0,
                                  device_storage_path=str(self.path))
        with self.assertRaises(BacnetError) as raised:
            await renumbered.start()
        message = str(raised.exception)
        for part in (str(self.path), f",{INSTANCE}", f",{INSTANCE + 1}",
                     "remove or move it", "a path of its own"):
            self.assertIn(part, message)

    async def test_a_relative_path_is_resolved_at_construction(self) -> None:
        home = os.getcwd()
        self.addCleanup(os.chdir, home)
        os.chdir(self.path.parent.parent)
        server = BACnetServer(INSTANCE, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1",
                              device_storage_path=os.path.join("state", "device.tags"))
        elsewhere = tempfile.TemporaryDirectory()
        self.addCleanup(elsewhere.cleanup)
        os.chdir(elsewhere.name)
        await server.start()
        try:
            await self.peer_writes(server, (P.DESCRIPTION, LOBBY))
        finally:
            await server.stop()
            # Leave the directory before it is removed: Windows refuses to
            # delete a process's working directory.
            os.chdir(home)
        self.assertTrue(self.path.exists())
        self.assertFalse(Path(elsewhere.name, "state").exists())

    async def test_a_write_whose_save_fails_is_refused(self) -> None:
        server = await self.started()
        try:
            await self.peer_writes(server, (P.DESCRIPTION, LOBBY))
            # A directory where the save writes its temporary file.
            os.mkdir(f"{self.path}.tmp")
            with self.assertRaises(BacnetProtocolError) as raised:
                await self.peer_writes(
                    server, (P.DESCRIPTION, PropertyValue.character_string("Atrium")))
            self.assertEqual(raised.exception.error_class, ErrorClass.DEVICE.to_raw())
            self.assertEqual(raised.exception.error_code,
                             ErrorCode.OPERATIONAL_PROBLEM.to_raw())
            self.assertEqual(await self.read(server, P.DESCRIPTION), LOBBY)
        finally:
            await server.stop()

    async def test_an_endpoint_started_again_on_the_path_serves_the_description(self) -> None:
        def endpoint() -> BipEndpoint:
            return BipEndpoint(INSTANCE, interface="127.0.0.1", port=0,
                               broadcast_address="127.0.0.1", mutation_policy="permissive",
                               device_storage_path=str(self.path))

        async with endpoint() as owner:
            address = await owner.local_address()
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                await client.write_property(address, DEVICE, P.DESCRIPTION, LOBBY)
        async with endpoint() as owner:
            server = await owner.server()
            self.assertEqual(await server.read_property(DEVICE, P.DESCRIPTION), LOBBY)

    def test_the_keyword(self) -> None:
        # As an object's storage_path= refuses one.
        with self.assertRaisesRegex(BacnetError, "path must not be empty"):
            BACnetServer(INSTANCE, device_storage_path="")
        with self.assertRaisesRegex(BacnetError, "path must not be empty"):
            BipEndpoint(INSTANCE, device_storage_path="")
        with self.assertRaises(TypeError):
            BACnetServer(INSTANCE, device_storage_path=7)
        for owner in (BACnetServer, BipEndpoint):
            parameter = inspect.signature(owner).parameters["device_storage_path"]
            self.assertIs(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
            self.assertIsNone(parameter.default)

    def test_installed_stub_shape(self) -> None:
        spec = importlib.util.find_spec("rusty_bacnet")
        assert spec is not None and spec.origin is not None
        origin = Path(spec.origin)
        candidates = [origin.with_suffix(".pyi"), origin.parent / "rusty_bacnet.pyi"]
        stub = next(path for path in candidates if path.exists())
        classes = {node.name: node for node in ast.parse(stub.read_text()).body
                   if isinstance(node, ast.ClassDef)}
        for owner in (BACnetServer, BipEndpoint, ScEndpoint, MstpEndpoint):
            with self.subTest(owner=owner.__name__):
                init = next(node for node in classes[owner.__name__].body
                            if isinstance(node, ast.FunctionDef) and node.name == "__init__")
                keywords = {arg.arg: (ast.unparse(arg.annotation), ast.unparse(default))
                            for arg, default in zip(init.args.kwonlyargs, init.args.kw_defaults)
                            if default is not None}
                self.assertEqual(keywords["device_storage_path"], ("Optional[str]", "None"))
                self.assertIn("device_storage_path",
                              inspect.signature(owner).parameters)


if __name__ == "__main__":
    unittest.main()
