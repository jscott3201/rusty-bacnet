"""Installed-artifact tests for Lighting Output Present_Value levels (#1385).

A level above 0.0 and below 1.0, written locally or over the network, is
stored as 1.0: Present_Value, the priority slot and Tracking_Value all read
1.0. 0.0 and 1.0 to 100.0 are stored as written, and a level outside 0.0 to
100.0 is refused with VALUE_OUT_OF_RANGE, but for the warn values -1.0, -2.0
and -3.0 (#1384). A Relinquish_Default in the same gap is stored as 1.0 too.
"""

from __future__ import annotations

import struct
import unittest

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

PV = PropertyIdentifier.PRESENT_VALUE
PA = PropertyIdentifier.PRIORITY_ARRAY
TV = PropertyIdentifier.TRACKING_VALUE
RD = PropertyIdentifier.RELINQUISH_DEFAULT
LO = ObjectIdentifier(ObjectType.LIGHTING_OUTPUT, 1)
# The smallest positive REAL and the largest REAL below 1.0.
JUST_ABOVE_OFF = struct.unpack(">f", bytes([0, 0, 0, 1]))[0]
JUST_BELOW_ONE = struct.unpack(">f", bytes([0x3F, 0x7F, 0xFF, 0xFF]))[0]


def make_server() -> BACnetServer:
    server = BACnetServer(
        device_instance=1_385_001,
        device_name="Lighting Present Value Artifact Test",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )
    server.add_lighting_output(1, "LO-1")
    return server


class LightingPresentValueTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.server = make_server()
        await self.server.start()

        async def stop_server() -> None:
            await self.server.stop()

        self.addAsyncCleanup(stop_server)
        self.address = await self.server.local_address()
        self.client = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
        await self.client.__aenter__()

        async def stop_client() -> None:
            await self.client.__aexit__(None, None, None)

        self.addAsyncCleanup(stop_client)

    def writers(self):
        """Ways to write Present_Value at priority 8: over the network and locally."""

        async def network(value: PropertyValue) -> None:
            await self.client.write_property(self.address, LO, PV, value, priority=8)

        async def local(value: PropertyValue) -> None:
            await self.server.write_property_local(LO, PV, value, priority=8, source_object=None)

        return (("network", network), ("local", local))

    async def levels_at_8(self) -> list[PropertyValue]:
        """Present_Value, Priority_Array[8] and Tracking_Value, read locally and remotely."""
        local = [
            await self.server.read_property(LO, PV),
            await self.server.read_property(LO, PA, 8),
            await self.server.read_property(LO, TV),
        ]
        remote = [
            await self.client.read_property(self.address, LO, PV),
            await self.client.read_property(self.address, LO, PA, 8),
            await self.client.read_property(self.address, LO, TV),
        ]
        return local + remote

    async def relinquish(self) -> None:
        await self.server.write_property_local(
            LO, PV, PropertyValue.null(), priority=8, source_object=None
        )
        self.assertEqual(await self.server.read_property(LO, PV), PropertyValue.real(0.0))

    async def test_level_below_one_percent_is_stored_as_one_percent(self) -> None:
        for how, write in self.writers():
            for level in (JUST_ABOVE_OFF, 0.5, JUST_BELOW_ONE):
                with self.subTest(how=how, level=level):
                    await write(PropertyValue.real(level))
                    self.assertEqual(await self.levels_at_8(), [PropertyValue.real(1.0)] * 6)
                    await self.relinquish()

    async def test_off_and_one_percent_up_are_stored_as_written(self) -> None:
        for how, write in self.writers():
            for level in (1.0, 50.0, 100.0, 0.0):
                with self.subTest(how=how, level=level):
                    await write(PropertyValue.real(level))
                    self.assertEqual(await self.levels_at_8(), [PropertyValue.real(level)] * 6)

    async def test_level_outside_the_range_is_refused_and_changes_nothing(self) -> None:
        await self.server.write_property_local(
            LO, PV, PropertyValue.real(0.5), priority=8, source_object=None
        )
        # -1.0, -2.0 and -3.0 are the warn commands (#1384); -1.5 is no value.
        for level in (-JUST_ABOVE_OFF, -1.5, 100.5):
            with self.subTest(level=level):
                value = PropertyValue.real(level)
                with self.assertRaises(BacnetProtocolError) as network:
                    await self.client.write_property(self.address, LO, PV, value, priority=8)
                with self.assertRaises(BacnetProtocolError) as local:
                    await self.server.write_property_local(
                        LO, PV, value, priority=8, source_object=None
                    )
                for raised in (network.exception, local.exception):
                    self.assertEqual(raised.error_class, ErrorClass.PROPERTY.to_raw())
                    self.assertEqual(raised.error_code, ErrorCode.VALUE_OUT_OF_RANGE.to_raw())
                self.assertEqual(await self.levels_at_8(), [PropertyValue.real(1.0)] * 6)

    async def test_relinquish_default_below_one_percent_is_stored_as_one_percent(self) -> None:
        await self.client.write_property(self.address, LO, RD, PropertyValue.real(0.5))
        self.assertEqual(await self.server.read_property(LO, RD), PropertyValue.real(1.0))
        # With every slot empty, Present_Value and Tracking_Value come from it.
        self.assertEqual(await self.server.read_property(LO, PV), PropertyValue.real(1.0))
        self.assertEqual(await self.server.read_property(LO, TV), PropertyValue.real(1.0))
        await self.server.write_property_local(
            LO, RD, PropertyValue.real(0.0), source_object=None
        )
        self.assertEqual(await self.server.read_property(LO, PV), PropertyValue.real(0.0))

    async def test_a_local_command_names_the_device_as_its_value_source(self) -> None:
        # Lighting Output serves Value_Source and Last_Command_Time (#1866).
        before = await self.server.read_property(LO, PropertyIdentifier.LAST_COMMAND_TIME)
        source = await self.server.read_property(LO, PropertyIdentifier.VALUE_SOURCE)
        self.assertEqual((source.tag, source.value), ("value_source", None))
        await self.server.write_property_local(
            LO, PV, PropertyValue.real(40.0), priority=8, source_object=None
        )
        source = await self.server.read_property(LO, PropertyIdentifier.VALUE_SOURCE)
        self.assertEqual(
            (source.tag, source.value),
            ("value_source", ObjectIdentifier(ObjectType.DEVICE, 1_385_001)),
        )
        after = await self.server.read_property(LO, PropertyIdentifier.LAST_COMMAND_TIME)
        self.assertEqual(after.tag, "timestamp")
        self.assertNotEqual(after.value, before.value)


if __name__ == "__main__":
    unittest.main()
