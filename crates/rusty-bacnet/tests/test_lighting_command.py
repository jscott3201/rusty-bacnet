"""Installed-artifact tests for Lighting Output's Lighting_Command (#1263, #1384).

Lighting_Command reads as a ``tagged`` value holding the context-tagged
BACnetLightingCommand (#2618), operation NONE until written. It takes writes in that
encoding, locally, over the network and from a Channel. An ``octet_string``,
the form it used to take, is refused with INVALID_DATA_TYPE, and a command its
operation can't take with VALUE_OUT_OF_RANGE. A command taken is carried out:
a FADE_TO puts its level in Present_Value at once and moves Tracking_Value
there over its fade time.
"""

from __future__ import annotations

import asyncio
import time
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

LC = PropertyIdentifier.LIGHTING_COMMAND
PV = PropertyIdentifier.PRESENT_VALUE
TV = PropertyIdentifier.TRACKING_VALUE
IN_PROGRESS = PropertyIdentifier.IN_PROGRESS
LO = ObjectIdentifier(ObjectType.LIGHTING_OUTPUT, 1)
CH = ObjectIdentifier(ObjectType.CHANNEL, 1)
# Write_Status SUCCESSFUL.
SUCCESSFUL = 2
# Operation [0] NONE.
NONE = bytes([0x09, 0x00])
# FADE_TO (1), target level [1] 50.0 % (REAL 0x42480000), priority [5] 8.
FADE = bytes([0x09, 0x01, 0x1C, 0x42, 0x48, 0x00, 0x00, 0x59, 0x08])
# STOP (10) at priority [5] 3.
STOP = bytes([0x09, 0x0A, 0x59, 0x03])
# FADE_TO (1), target level [1] 80.0 % (0x42A00000), fade time [4] 500 ms
# (0x01F4), priority [5] 8.
FADE_80 = bytes([0x09, 0x01, 0x1C, 0x42, 0xA0, 0x00, 0x00, 0x4A, 0x01, 0xF4, 0x59, 0x08])
# In_Progress IDLE.
IDLE = 0


def make_server() -> BACnetServer:
    server = BACnetServer(
        device_instance=1_263_001,
        device_name="Lighting Command Artifact Test",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )
    server.add_lighting_output(1, "LO-1")
    # CH-1 passes its value on to LO-1's Lighting_Command.
    server.add_channel(1, "CH-1", 11, members=[(LO, LC)])
    return server


class LightingCommandTests(unittest.IsolatedAsyncioTestCase):
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

    async def read_both(self) -> list[PropertyValue]:
        return [
            await self.server.read_property(LO, LC),
            await self.client.read_property(self.address, LO, LC),
        ]

    async def test_reads_none_until_written(self) -> None:
        for value in await self.read_both():
            self.assertEqual(value, PropertyValue.tagged(NONE))

    async def test_commands_write_and_read_in_their_encoding(self) -> None:
        await self.client.write_property(
            self.address, LO, LC, PropertyValue.application_data(FADE)
        )
        for value in await self.read_both():
            self.assertEqual(value.tag, "tagged")
            self.assertEqual(value.octets, FADE)
        await self.server.write_property_local(
            LO, LC, PropertyValue.application_data(STOP), source_object=None
        )
        for value in await self.read_both():
            self.assertEqual(value.octets, STOP)
        # The value read writes back locally unchanged.
        value = await self.server.read_property(LO, LC)
        await self.server.write_property_local(LO, LC, value, source_object=None)
        self.assertEqual(await self.server.read_property(LO, LC), value)

    async def assert_refused(self, value: PropertyValue, code: ErrorCode) -> None:
        with self.assertRaises(BacnetProtocolError) as network:
            await self.client.write_property(self.address, LO, LC, value)
        with self.assertRaises(BacnetProtocolError) as local:
            await self.server.write_property_local(LO, LC, value, source_object=None)
        for raised in (network.exception, local.exception):
            self.assertEqual(raised.error_class, ErrorClass.PROPERTY.to_raw())
            self.assertEqual(raised.error_code, code.to_raw())

    async def test_octet_string_is_refused_and_changes_nothing(self) -> None:
        await self.assert_refused(PropertyValue.octet_string(FADE), ErrorCode.INVALID_DATA_TYPE)
        self.assertEqual(
            await self.server.read_property(LO, LC), PropertyValue.tagged(NONE)
        )

    async def test_command_its_operation_cannot_take_is_out_of_range(self) -> None:
        # NONE can't be written, and FADE_TO needs a target level.
        for octets in (NONE, bytes([0x09, 0x01, 0x59, 0x08])):
            with self.subTest(octets=octets.hex()):
                await self.assert_refused(
                    PropertyValue.application_data(octets), ErrorCode.VALUE_OUT_OF_RANGE
                )
        self.assertEqual(
            await self.server.read_property(LO, LC), PropertyValue.tagged(NONE)
        )

    async def test_a_fade_written_over_the_network_runs_to_its_level(self) -> None:
        await self.client.write_property(
            self.address, LO, LC, PropertyValue.application_data(FADE_80)
        )
        # The level is in Present_Value at once.
        self.assertEqual(await self.server.read_property(LO, PV), PropertyValue.real(80.0))
        # Tracking_Value gets there once the fade has run; In_Progress is IDLE.
        deadline = time.monotonic() + 10.0
        while (await self.server.read_property(LO, IN_PROGRESS)).value != IDLE:
            if time.monotonic() > deadline:
                raise AssertionError("the fade never finished")
            await asyncio.sleep(0.05)
        for value in (
            await self.server.read_property(LO, TV),
            await self.client.read_property(self.address, LO, TV),
        ):
            self.assertEqual(value, PropertyValue.real(80.0))

    async def test_present_value_warn_off_turns_the_light_off(self) -> None:
        # Blink_Warn_Enable is FALSE, so -3.0 (WARN_OFF) writes 0.0 at once.
        await self.client.write_property(
            self.address, LO, PV, PropertyValue.real(60.0), priority=8
        )
        await self.client.write_property(
            self.address, LO, PV, PropertyValue.real(-3.0), priority=8
        )
        self.assertEqual(
            await self.server.read_property(LO, PropertyIdentifier.PRIORITY_ARRAY, 8),
            PropertyValue.real(0.0),
        )
        self.assertEqual(await self.server.read_property(LO, PV), PropertyValue.real(0.0))

    async def test_channel_passes_a_lighting_command_on(self) -> None:
        # A channel value frames the command in context tag 0.
        framed = b"\x0e" + FADE + b"\x0f"
        await self.client.write_property(
            self.address, CH, PropertyIdentifier.PRESENT_VALUE,
            PropertyValue.application_data(framed),
        )
        deadline = time.monotonic() + 10.0
        status = PropertyIdentifier.WRITE_STATUS
        while (await self.server.read_property(CH, status)).value != SUCCESSFUL:
            if time.monotonic() > deadline:
                raise AssertionError("the distribution never succeeded")
            await asyncio.sleep(0.01)
        self.assertEqual(
            await self.server.read_property(LO, LC), PropertyValue.tagged(FADE)
        )


if __name__ == "__main__":
    unittest.main()
