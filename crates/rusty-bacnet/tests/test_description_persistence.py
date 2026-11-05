"""A written Description kept in the files of the builders whose objects
have a snapshot format of their own (#2364): the three logs, Notification
Class, Notification Forwarder and Access Rights. A server started on the same
files serves it again; a log's ``clear_written_reporting=True`` drops it
from the files; one past 16 KiB is refused only while it would be kept.
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from rusty_bacnet import (
    BacnetProtocolError,
    BACnetServer,
    ErrorClass,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

P = PropertyIdentifier
LOBBY = PropertyValue.character_string("Lobby")
EMPTY = PropertyValue.character_string("")
TOO_LONG = PropertyValue.character_string("d" * (16 * 1024 + 1))
TREND = ObjectIdentifier(ObjectType.TREND_LOG, 1)
MULTIPLE = ObjectIdentifier(ObjectType.TREND_LOG_MULTIPLE, 1)
EVENT = ObjectIdentifier(ObjectType.EVENT_LOG, 1)
CLASS = ObjectIdentifier(ObjectType.NOTIFICATION_CLASS, 1)
FORWARDER = ObjectIdentifier(ObjectType.NOTIFICATION_FORWARDER, 1)
RIGHTS = ObjectIdentifier(ObjectType.ACCESS_RIGHTS, 1)
LOGS = (TREND, MULTIPLE, EVENT)
OBJECTS = LOGS + (CLASS, FORWARDER, RIGHTS)


class KeptDescriptionTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.state = Path(directory.name)

    def path(self, name: str) -> str:
        return str(self.state / name)

    async def start(self, clear: bool = False, stored: bool = True) -> BACnetServer:
        path = self.path if stored else (lambda _name: None)
        server = BACnetServer(2_364_002, interface="127.0.0.1", port=0)
        server.add_trend_log(
            1, "TL-1", storage_path=path("tl-1"), clear_written_reporting=clear
        )
        server.add_trend_log_multiple(
            1, "TLM-1", storage_path=path("tlm-1"), clear_written_reporting=clear
        )
        server.add_event_log(
            1, "EL-1", storage_path=path("el-1"), clear_written_reporting=clear
        )
        server.add_notification_class(1, "NC-1", storage_path=path("nc-1"))
        server.add_notification_forwarder(1, "NF-1", storage_path=path("nf-1"))
        server.add_access_rights(1, "AR-1", storage_path=path("ar-1"))
        await server.start()
        return server

    async def write(self, server: BACnetServer, oid: ObjectIdentifier, value) -> None:
        await server.write_property_local(oid, P.DESCRIPTION, value, source_object=None)

    async def test_a_written_description_survives_a_restart(self) -> None:
        server = await self.start()
        try:
            for oid in OBJECTS:
                await self.write(server, oid, LOBBY)
                # A NULL changes nothing.
                await self.write(server, oid, PropertyValue.null())
        finally:
            await server.stop()

        server = await self.start()
        try:
            for oid in OBJECTS:
                with self.subTest(oid=oid):
                    self.assertEqual(await server.read_property(oid, P.DESCRIPTION), LOBBY)
        finally:
            await server.stop()

        # Clearing a log's written settings drops its Description from the
        # files too; as for the other settings, the log serves it until the
        # next start. The other three have no clear.
        server = await self.start(clear=True)
        try:
            for oid in OBJECTS:
                with self.subTest(clearing=oid):
                    self.assertEqual(await server.read_property(oid, P.DESCRIPTION), LOBBY)
        finally:
            await server.stop()

        server = await self.start()
        try:
            for oid in OBJECTS:
                with self.subTest(cleared=oid):
                    expected = EMPTY if oid in LOGS else LOBBY
                    self.assertEqual(await server.read_property(oid, P.DESCRIPTION), expected)
        finally:
            await server.stop()

    async def test_a_description_past_16_kib_is_refused_only_when_kept(self) -> None:
        server = await self.start()
        try:
            for oid in OBJECTS:
                with self.subTest(oid=oid):
                    with self.assertRaises(BacnetProtocolError) as raised:
                        await self.write(server, oid, TOO_LONG)
                    self.assertEqual(raised.exception.error_class, ErrorClass.RESOURCES.to_raw())
                    self.assertEqual(
                        raised.exception.error_code,
                        ErrorCode.NO_SPACE_TO_WRITE_PROPERTY.to_raw(),
                    )
                    self.assertEqual(await server.read_property(oid, P.DESCRIPTION), EMPTY)
        finally:
            await server.stop()

        server = await self.start(stored=False)
        try:
            for oid in OBJECTS:
                with self.subTest(in_memory=oid):
                    await self.write(server, oid, TOO_LONG)
                    self.assertEqual(await server.read_property(oid, P.DESCRIPTION), TOO_LONG)
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
