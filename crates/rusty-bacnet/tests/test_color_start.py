"""Color and Color Temperature outputs at the server's start (#1526): driven
to a non-zero default, a zero default keeping the Present_Value kept in
storage_path, and In_Progress NOT_CONTROLLED until the output is driven."""
import tempfile
import unittest
from pathlib import Path

from rusty_bacnet import (BACnetServer, BacnetError, BacnetProtocolError, ErrorCode,
                          ObjectIdentifier, ObjectType, PropertyIdentifier, PropertyValue)

P = PropertyIdentifier
DEVICE = 9526
COLOR = ObjectIdentifier(ObjectType.COLOR, 1)
WARM = ObjectIdentifier(ObjectType.COLOR_TEMPERATURE, 1)
IDLE = 0
NOT_CONTROLLED = 3


def new_server():
    return BACnetServer(DEVICE, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")


def xy(x, y):
    return PropertyValue.list([PropertyValue.real(x), PropertyValue.real(y)])


class ColorStartTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.directory = Path(directory.name)

    async def read(self, server, oid, prop):
        return (await server.read_property(oid, prop)).value

    async def assert_xy(self, server, expected):
        x, y = await self.read(server, COLOR, P.PRESENT_VALUE)
        self.assertAlmostEqual(x, expected[0], places=6)
        self.assertAlmostEqual(y, expected[1], places=6)

    async def test_a_non_zero_default_is_driven_at_start(self):
        server = new_server()
        server.add_color(1, "Color", default_color=(0.64, 0.33))
        server.add_color_temperature(1, "Warm", default_color_temperature=9000,
                                     min_pres_value=2000, max_pres_value=6500)
        await server.start()
        try:
            await self.assert_xy(server, (0.64, 0.33))
            self.assertEqual(await self.read(server, COLOR, P.IN_PROGRESS), IDLE)
            # Clamped to the limits, as a written default is.
            self.assertEqual(await self.read(server, WARM, P.DEFAULT_COLOR_TEMPERATURE), 6500)
            self.assertEqual(await self.read(server, WARM, P.PRESENT_VALUE), 6500)
            self.assertEqual(await self.read(server, WARM, P.IN_PROGRESS), IDLE)
        finally:
            await server.stop()

    async def test_a_zero_default_without_storage_waits_for_a_write(self):
        server = new_server()
        server.add_color_temperature(1, "Warm", default_color_temperature=0)
        await server.start()
        try:
            self.assertEqual(await self.read(server, WARM, P.IN_PROGRESS), NOT_CONTROLLED)
            await server.write_property_local(WARM, P.PRESENT_VALUE, PropertyValue.unsigned(3000),
                                              source_object=None)
            self.assertEqual(await self.read(server, WARM, P.IN_PROGRESS), IDLE)
        finally:
            await server.stop()

    async def test_a_zero_default_keeps_the_stored_value_across_a_restart(self):
        color_path = str(self.directory / "color-1")
        warm_path = str(self.directory / "warm-1")

        def build():
            server = new_server()
            # The stored value wins over present_value once there is one.
            server.add_color(1, "Color", default_color=(0.0, 0.0), storage_path=color_path,
                             present_value=(0.3, 0.3))
            server.add_color_temperature(1, "Warm", default_color_temperature=0,
                                         storage_path=warm_path, present_value=4500)
            return server

        server = build()
        await server.start()
        try:
            await self.assert_xy(server, (0.3, 0.3))
            await server.write_property_local(COLOR, P.PRESENT_VALUE, xy(0.15, 0.06),
                                              source_object=None)
            await server.write_property_local(WARM, P.PRESENT_VALUE, PropertyValue.unsigned(2700),
                                              source_object=None)
        finally:
            await server.stop()

        server = build()
        await server.start()
        try:
            await self.assert_xy(server, (0.15, 0.06))
            self.assertEqual(await self.read(server, WARM, P.PRESENT_VALUE), 2700)
            for oid in (COLOR, WARM):
                self.assertEqual(await self.read(server, oid, P.IN_PROGRESS), IDLE)
        finally:
            await server.stop()

    async def test_a_non_zero_default_wins_over_present_value_on_every_boot(self):
        path = str(self.directory / "warm-2")
        for boot in ("first", "later"):
            with self.subTest(boot=boot):
                server = new_server()
                server.add_color_temperature(1, "Warm", present_value=5000,
                                             default_color_temperature=3000,
                                             storage_path=path)
                await server.start()
                try:
                    self.assertEqual(await self.read(server, WARM, P.PRESENT_VALUE), 3000)
                    await server.write_property_local(WARM, P.PRESENT_VALUE,
                                                      PropertyValue.unsigned(6000),
                                                      source_object=None)
                finally:
                    await server.stop()

    async def test_lighting_color_is_none_while_the_colour_object_is_undriven(self):
        server = new_server()
        server.add_color_temperature(1, "Warm", default_color_temperature=0)
        server.add_lighting_output(1, "Light", color_reference=WARM)
        light = ObjectIdentifier(ObjectType.LIGHTING_OUTPUT, 1)
        await server.start()
        try:
            self.assertIsNone(await server.lighting_color(light))
            await server.write_property_local(WARM, P.PRESENT_VALUE, PropertyValue.unsigned(2700),
                                              source_object=None)
            self.assertEqual(await server.lighting_color(light), (WARM, 2700))
        finally:
            await server.stop()

    def test_bad_defaults_and_files_are_refused(self):
        server = new_server()
        with self.assertRaises(BacnetProtocolError) as raised:
            server.add_color(1, "Bad", default_color=(1.5, 0.0))
        self.assertEqual(raised.exception.error_code, ErrorCode.VALUE_OUT_OF_RANGE.to_raw())
        with self.assertRaises(BacnetProtocolError):
            server.add_color_temperature(1, "Bad", default_color_temperature=999)
        path = self.directory / "corrupt"
        path.write_bytes(b"not a colour file")
        with self.assertRaisesRegex(BacnetError, "has no valid header"):
            server.add_color(1, "Corrupt", storage_path=str(path))
        with self.assertRaisesRegex(BacnetError, "path must not be empty"):
            server.add_color_temperature(1, "Empty", storage_path="")
        self.assertEqual(server._pending_registration_count(), 0)


if __name__ == "__main__":
    unittest.main()
