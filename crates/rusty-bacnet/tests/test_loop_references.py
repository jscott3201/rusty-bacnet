"""Installed-artifact tests for the Loop and Pulse Converter references (#1312).

Controlled_Variable_Reference, Manipulated_Variable_Reference and Input_Reference
read as a ``tagged`` value holding the context-tagged
BACnetObjectPropertyReference (#2618), and Setpoint_Reference holding the
BACnetSetpointReference. They take writes in those encodings, locally and over
the network, and refuse the flat list they used to read as with
INVALID_DATA_TYPE. Unset, the first three read as a reference to the reserved
instance 4194303, which clears them when written, and a NULL written to any of
the four succeeds and changes nothing (#1417). A Pulse Converter's Reliability
reads CONFIGURATION_ERROR while Input_Reference names a property it can't count
from (#1341).
"""

from __future__ import annotations

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

P = PropertyIdentifier
LOOP = ObjectIdentifier(ObjectType.LOOP, 1)
PC = ObjectIdentifier(ObjectType.PULSE_CONVERTER, 1)
AI_7 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 7)
# [0] analog-input 7, [1] present-value (85).
AI_7_PV = bytes([0x0C, 0x00, 0x00, 0x00, 0x07, 0x19, 0x55])
# The same reference inside a BACnetSetpointReference: opening and closing tag 0.
AI_7_PV_FRAMED = b"\x0e" + AI_7_PV + b"\x0f"
# The unset forms (#1417): [0] analog-input, analog-output or accumulator
# 4194303, [1] present-value.
UNSET = {
    P.CONTROLLED_VARIABLE_REFERENCE: bytes([0x0C, 0x00, 0x3F, 0xFF, 0xFF, 0x19, 0x55]),
    P.MANIPULATED_VARIABLE_REFERENCE: bytes([0x0C, 0x00, 0x7F, 0xFF, 0xFF, 0x19, 0x55]),
    P.INPUT_REFERENCE: bytes([0x0C, 0x05, 0xFF, 0xFF, 0xFF, 0x19, 0x55]),
}
FLAT = PropertyValue.list(
    [PropertyValue.object_identifier(AI_7), PropertyValue.enumerated(85)]
)


def make_server() -> BACnetServer:
    server = BACnetServer(
        device_instance=1_312_001,
        device_name="Loop Reference Artifact Test",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )
    server.add_loop(1, "LOOP-1")
    server.add_pulse_converter(1, "PC-1", 95)
    server.add_accumulator(1, "ACC-1", 95)
    return server


class LoopReferenceTests(unittest.IsolatedAsyncioTestCase):
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

    async def test_unset_references_read_the_reserved_instance_or_the_empty_setpoint(
        self,
    ) -> None:
        for oid, prop in ((LOOP, P.CONTROLLED_VARIABLE_REFERENCE),
                          (LOOP, P.MANIPULATED_VARIABLE_REFERENCE),
                          (PC, P.INPUT_REFERENCE)):
            with self.subTest(property=prop):
                for value in (await self.server.read_property(oid, prop),
                              await self.client.read_property(self.address, oid, prop)):
                    self.assertEqual(value.tag, "tagged")
                    self.assertEqual(value.octets, UNSET[prop])
        # A Setpoint_Reference without a reference has no octets.
        self.assertEqual(
            await self.server.read_property(LOOP, P.SETPOINT_REFERENCE), PropertyValue.list([])
        )

    async def test_references_read_and_write_in_their_encodings(self) -> None:
        for oid, prop, octets in ((LOOP, P.CONTROLLED_VARIABLE_REFERENCE, AI_7_PV),
                                  (LOOP, P.MANIPULATED_VARIABLE_REFERENCE, AI_7_PV),
                                  (LOOP, P.SETPOINT_REFERENCE, AI_7_PV_FRAMED),
                                  (PC, P.INPUT_REFERENCE, AI_7_PV)):
            with self.subTest(property=prop):
                await self.client.write_property(
                    self.address, oid, prop, PropertyValue.application_data(octets)
                )
                for value in (await self.server.read_property(oid, prop),
                              await self.client.read_property(self.address, oid, prop)):
                    self.assertEqual(value.tag, "tagged")
                    self.assertEqual(value.octets, octets)
                # The value read writes back locally unchanged.
                value = await self.server.read_property(oid, prop)
                await self.server.write_property_local(oid, prop, value, source_object=None)
                self.assertEqual(await self.server.read_property(oid, prop), value)

    async def test_flat_list_is_refused_and_changes_nothing(self) -> None:
        for oid, prop in ((LOOP, P.CONTROLLED_VARIABLE_REFERENCE),
                          (LOOP, P.MANIPULATED_VARIABLE_REFERENCE),
                          (LOOP, P.SETPOINT_REFERENCE),
                          (PC, P.INPUT_REFERENCE)):
            with self.subTest(property=prop):
                before = await self.server.read_property(oid, prop)
                with self.assertRaises(BacnetProtocolError) as network:
                    await self.client.write_property(self.address, oid, prop, FLAT)
                with self.assertRaises(BacnetProtocolError) as local:
                    await self.server.write_property_local(oid, prop, FLAT, source_object=None)
                for raised in (network.exception, local.exception):
                    self.assertEqual(raised.error_class, ErrorClass.PROPERTY.to_raw())
                    self.assertEqual(raised.error_code, ErrorCode.INVALID_DATA_TYPE.to_raw())
                self.assertEqual(await self.server.read_property(oid, prop), before)

    async def test_the_unset_forms_clear_and_null_changes_nothing(self) -> None:
        for oid, prop, octets in ((LOOP, P.CONTROLLED_VARIABLE_REFERENCE, AI_7_PV),
                                  (LOOP, P.MANIPULATED_VARIABLE_REFERENCE, AI_7_PV),
                                  (LOOP, P.SETPOINT_REFERENCE, AI_7_PV_FRAMED),
                                  (PC, P.INPUT_REFERENCE, AI_7_PV)):
            with self.subTest(property=prop):
                await self.client.write_property(
                    self.address, oid, prop, PropertyValue.application_data(octets)
                )
                # None of these has a NULL in its datatype or is commandable,
                # so a NULL succeeds and leaves it as it is (#1396, #1417),
                # over the network and locally.
                await self.client.write_property(self.address, oid, prop, PropertyValue.null())
                await self.server.write_property_local(
                    oid, prop, PropertyValue.null(), source_object=None
                )
                value = await self.server.read_property(oid, prop)
                self.assertEqual(value.octets, octets)
                # The unset form clears it: the reserved instance, or for
                # Setpoint_Reference the value with no octets.
                unset = UNSET.get(prop)
                cleared = (PropertyValue.application_data(unset) if unset is not None
                           else PropertyValue.list([]))
                await self.client.write_property(self.address, oid, prop, cleared)
                value = await self.server.read_property(oid, prop)
                if unset is not None:
                    self.assertEqual(value.octets, unset)
                else:
                    self.assertEqual(value, PropertyValue.list([]))


class PulseConverterInputTests(unittest.IsolatedAsyncioTestCase):
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

    async def test_reliability_follows_what_input_reference_names(self) -> None:
        # Reliability raw values: NO_FAULT_DETECTED 0, CONFIGURATION_ERROR 10;
        # Status_Flags FAULT is the second of four bits.
        accumulator_pv = bytes([0x0C, 0x05, 0xC0, 0x00, 0x01, 0x19, 0x55])
        for octets, reliability, fault, what in (
            (AI_7_PV, 10, True, "a missing object"),
            (accumulator_pv, 0, False, "an Accumulator's Unsigned Present_Value"),
            (bytes([0x0C, 0x03, 0x00, 0x00, 0x01, 0x19, 0x55]), 10, True, "a Loop's REAL"),
            (UNSET[P.INPUT_REFERENCE], 0, False, "no reference"),
        ):
            with self.subTest(reference=what):
                await self.client.write_property(
                    self.address, PC, P.INPUT_REFERENCE, PropertyValue.application_data(octets)
                )
                value = await self.client.read_property(self.address, PC, P.RELIABILITY)
                self.assertEqual(value.value, reliability)
                flags = await self.client.read_property(self.address, PC, P.STATUS_FLAGS)
                self.assertEqual(
                    flags, PropertyValue.bit_string(4, b"\x40" if fault else b"\x00")
                )


if __name__ == "__main__":
    unittest.main()
