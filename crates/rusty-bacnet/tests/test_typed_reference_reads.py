"""Installed native reads of single device object/property references (#1499)."""
import copy
import pickle
import unittest
from contextlib import asynccontextmanager

from rusty_bacnet import (BACnetClient, BACnetServer, BACnetTimeStamp, BacnetError,
                         BacnetProtocolError, ErrorClass, ErrorCode, ObjectIdentifier,
                         ObjectType, PropertyIdentifier, PropertyValue)
from typed_reference_peer import ReferencePeer, Target

P = PropertyIdentifier
O = ObjectType
TREND = ObjectIdentifier(O.TREND_LOG, 1)


class NativeReferenceReadTests(unittest.IsolatedAsyncioTestCase):
    async def test_unset_trend_log_reads_as_one_typed_property_reference(self):
        server = BACnetServer(9499, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1")
        server.add_trend_log(1, "Log")
        await server.start()
        try:
            value = await server.read_property(TREND, P.LOG_DEVICE_OBJECT_PROPERTY)
            self.assertEqual(value.tag, "device_object_property_reference")
            self.assertEqual(value.value, {
                "object_identifier": ObjectIdentifier(O.ANALOG_INPUT, 4194303),
                "property_identifier": P.PRESENT_VALUE,
                "property_array_index": None,
                "device_identifier": None,
            })
        finally:
            await server.stop()


# Object and property numbers are independently authored wire coordinates.
TARGETS = (
    Target(TREND, P.LOG_DEVICE_OBJECT_PROPERTY, (20 << 22) | 1, 132),
    Target(ObjectIdentifier(O.AVERAGING, 1), P.OBJECT_PROPERTY_REFERENCE, (18 << 22) | 1, 78),
    Target(ObjectIdentifier(O.EVENT_ENROLLMENT, 1), P.OBJECT_PROPERTY_REFERENCE, (9 << 22) | 1, 78),
    Target(ObjectIdentifier(O.ACCESS_POINT, 1), P.ACCESS_EVENT_CREDENTIAL, (33 << 22) | 1, 249),
    Target(ObjectIdentifier(O.LIFT, 1), P.ENERGY_METER_REF, (59 << 22) | 1, 461),
    Target(ObjectIdentifier(O.ESCALATOR, 1), P.ENERGY_METER_REF, (58 << 22) | 1, 461),
)
PROPERTY_TAG = "device_object_property_reference"
OBJECT_TAG = "device_object_reference"
REMOTE = ObjectIdentifier(O.DEVICE, 99)
RESERVED_DEVICE = ObjectIdentifier(O.DEVICE, 4194303)
LOCAL_PROPERTY = bytes.fromhex("0c000000071955")
UNSET_PROPERTY = bytes.fromhex("0c003fffff1955")


def reference(object_id, prop=P.PRESENT_VALUE, index=None, device=None):
    return {"object_identifier": object_id, "property_identifier": prop,
            "property_array_index": index, "device_identifier": device}


def vectors(target):
    if target in TARGETS[:3]:
        return PROPERTY_TAG, [
            (LOCAL_PROPERTY, reference(ObjectIdentifier(O.ANALOG_INPUT, 7))),
            (bytes.fromhex("0c00400008195729083c02000063"),
             reference(ObjectIdentifier(O.ANALOG_OUTPUT, 8), P.PRIORITY_ARRAY, 8, REMOTE)),
            (bytes.fromhex("0c0040000819572900"),
             reference(ObjectIdentifier(O.ANALOG_OUTPUT, 8), P.PRIORITY_ARRAY, 0)),
            (bytes.fromhex("0c0040000819572cffffffff"),
             reference(ObjectIdentifier(O.ANALOG_OUTPUT, 8), P.PRIORITY_ARRAY, 4294967295)),
            (UNSET_PROPERTY, reference(ObjectIdentifier(O.ANALOG_INPUT, 4194303))),
            (LOCAL_PROPERTY + bytes.fromhex("3c023fffff"),
             reference(ObjectIdentifier(O.ANALOG_INPUT, 7), device=RESERVED_DEVICE)),
        ]
    kind, word = (O.ACCESS_CREDENTIAL, 32 << 22) if target.prop == P.ACCESS_EVENT_CREDENTIAL else (O.ACCUMULATOR, 23 << 22)
    local = b"\x1c" + (word | 7).to_bytes(4, "big")
    return OBJECT_TAG, [
        (local, ObjectIdentifier(kind, 7)),
        (bytes.fromhex("0c02000063") + local, (REMOTE, ObjectIdentifier(kind, 7))),
        (b"\x1c" + (word | 4194303).to_bytes(4, "big"), ObjectIdentifier(kind, 4194303)),
        (bytes.fromhex("0c023fffff") + local, (RESERVED_DEVICE, ObjectIdentifier(kind, 7))),
    ]


class PeerReferenceReadTests(unittest.IsolatedAsyncioTestCase):
    def assert_reference(self, value, tag, expected):
        self.assertIsInstance(value, PropertyValue)
        self.assertEqual(value.tag, tag)
        self.assertEqual(value.value, expected)
        if tag == PROPERTY_TAG:
            self.assertEqual(set(value.value), {"object_identifier", "property_identifier",
                                               "property_array_index", "device_identifier"})
            self.assertIsInstance(value.value["property_identifier"], PropertyIdentifier)

    async def test_all_six_single_mappings_reach_direct_device_batch_and_endpoint_reads(self):
        for target in TARGETS:
            tag, examples = vectors(target)
            raw, expected = examples[1]
            for route in ("rp", "rpm", "device_rp", "device_rpm", "batch_rp", "batch_rpm", "endpoint_rp", "endpoint_rpm"):
                with self.subTest(target=target, route=route):
                    value = await ReferencePeer(self, target).read(raw, route=route)
                    self.assert_reference(value, tag, expected)

    async def test_optional_members_reserved_ids_and_exact_wrapper_echo(self):
        for target in TARGETS:
            tag, examples = vectors(target)
            for raw, expected in examples:
                with self.subTest(target=target, raw=raw):
                    # Fake peer acceptance only proves the wrapper's original bytes.
                    value = await ReferencePeer(self, target).read(raw, echo=True)
                    self.assert_reference(value, tag, expected)
                    restored = pickle.loads(pickle.dumps(value))
                    self.assertEqual(restored, value)
                    self.assert_reference(restored, tag, expected)
                    self.assert_reference(copy.deepcopy(value), tag, expected)

    async def test_invalid_single_shapes_keep_generic_categories(self):
        for target in TARGETS:
            _, examples = vectors(target)
            raw = examples[0][0]
            for malformed in (bytes.fromhex("2c00000007"), raw + raw, raw + b"\x21\x07", b"\x72\x00\xff"):
                for route in ("rp", "rpm"):
                    with self.subTest(target=target, raw=malformed, route=route):
                        value = await ReferencePeer(self, target).read(malformed, route=route)
                        self.assertEqual((value.tag, value.value), ("application_data", malformed))
            for index in (0, 1):
                value = await ReferencePeer(self, target).read(raw, index)
                self.assertEqual((value.tag, value.value), ("application_data", raw))
            value = await ReferencePeer(self, target).read(b"\x21\x07")
            self.assertEqual((value.tag, value.value), ("unsigned", 7))

    async def test_rpm_keeps_typed_generic_and_embedded_error_rows_separate(self):
        for target in TARGETS:
            tag, examples = vectors(target)
            raw, expected = examples[0]
            wrong = bytes.fromhex("2c00000007")
            values = await ReferencePeer(self, target).read(raw, route="rpm", extra=(wrong, b"\x21\x07"))
            self.assert_reference(values[0], tag, expected)
            self.assertEqual((values[1].tag, values[1].value), ("application_data", wrong))
            self.assertEqual((values[2].tag, values[2].value), ("unsigned", 7))
            # The helper independently asserts the last DESCRIPTION error row.

    async def test_broken_service_framing_remains_a_read_error(self):
        for target in (TARGETS[0], TARGETS[3]):
            for malformed in (b"\x0c\x00\x00", b"\x0e\x1f"):
                for route in ("rp", "rpm"):
                    with self.subTest(target=target, route=route, raw=malformed):
                        with self.assertRaises(BacnetError) as raised:
                            await ReferencePeer(self, target).read(malformed, route=route)
                        self.assertIs(type(raised.exception), BacnetError)
        # Malformed inner RPM values rejected upstream are covered separately at
        # the Rust converter boundary, where that category remains raw bytes.

    async def test_trend_log_multiple_keeps_collection_and_indexed_element_shapes(self):
        target = Target(ObjectIdentifier(O.TREND_LOG_MULTIPLE, 1), P.LOG_DEVICE_OBJECT_PROPERTY, (27 << 22) | 1, 132)
        first, second = vectors(TARGETS[0])[1][:2]
        peer = ReferencePeer(self, target)
        for raw, expected in [(b"", []), (first[0], [first[1]]), (first[0] + second[0], [first[1], second[1]])]:
            for route in ("rp", "rpm"):
                value = await peer.read(raw, route=route, echo=True)
                self.assertEqual((value.tag, value.value), ("list", expected))
        value = await peer.read(second[0], 2, echo=True)
        self.assert_reference(value, PROPERTY_TAG, second[1])
        value = await peer.read(b"\x21\x02", 0)
        self.assertEqual((value.tag, value.value), ("unsigned", 2))


@asynccontextmanager
async def native_server():
    server = BACnetServer(9499, interface="127.0.0.1", port=0,
                          broadcast_address="127.0.0.1")
    server.add_analog_input(7, "Input", present_value=21.5)
    server.add_trend_log(1, "Log")
    server.add_averaging(1, "Average")
    server.add_event_enrollment(1, "Enrollment")
    server.add_access_point(1, "Access Point")
    server.add_lift(1, "Lift", 1)
    server.add_escalator(1, "Escalator")
    await server.start()
    try:
        async with BACnetClient(interface="127.0.0.1", port=0) as client:
            yield server, client, await server.local_address()
    finally:
        await server.stop()


class NativeReferenceRoutesTests(unittest.IsolatedAsyncioTestCase):
    async def assert_reads(self, server, client, address, target, tag, expected):
        local = await server.read_property(target.oid, target.prop)
        direct = await client.read_property(address, target.oid, target.prop)
        rpm = await client.read_property_multiple(address, [(target.oid, [(target.prop, None)])])
        row = rpm[0]["results"][0]
        self.assertIsNone(row["error"])
        for value in (local, direct, row["value"]):
            self.assertEqual((value.tag, value.value), (tag, expected))
        return local

    async def test_native_unset_forms_expose_reserved_identifiers_on_all_six_objects(self):
        async with native_server() as (server, client, address):
            for target in TARGETS:
                tag, examples = vectors(target)
                expected = examples[4 if tag == PROPERTY_TAG else 2][1]
                with self.subTest(target=target):
                    await self.assert_reads(server, client, address, target, tag, expected)

    async def test_existing_writable_references_accept_and_return_byte_backed_values(self):
        async with native_server() as (server, client, address):
            for target in TARGETS[:2]:
                # Running server has the Device clock needed by Trend's buffer purge.
                own_device = LOCAL_PROPERTY + b"\x3c" + ((8 << 22) | 9499).to_bytes(4, "big")
                await server.write_property_local(target.oid, target.prop,
                    PropertyValue.application_data(own_device), source_object=None)
                value = await self.assert_reads(server, client, address, target, PROPERTY_TAG,
                                               reference(ObjectIdentifier(O.ANALOG_INPUT, 7)))
                await client.write_property(address, target.oid, target.prop, value)
                await server.write_property_local(target.oid, target.prop, value, source_object=None)
                unset = PropertyValue.application_data(UNSET_PROPERTY)
                await client.write_property(address, target.oid, target.prop, unset)
                await self.assert_reads(server, client, address, target, PROPERTY_TAG,
                                        reference(ObjectIdentifier(O.ANALOG_INPUT, 4194303)))

    async def test_access_event_credential_reads_local_remote_and_unset_after_native_reports(self):
        async with native_server() as (server, client, address):
            target = TARGETS[3]
            badge = ObjectIdentifier(O.ACCESS_CREDENTIAL, 7)
            for sequence, credential in enumerate((badge, (REMOTE, badge), None), 1):
                await server.report_access_event_local(target.oid, 1, 7,
                    time=BACnetTimeStamp.sequence_number(sequence), credential=credential)
                expected = credential if credential is not None else ObjectIdentifier(O.ACCESS_CREDENTIAL, 4194303)
                await self.assert_reads(server, client, address, target, OBJECT_TAG, expected)

    async def test_read_only_reference_rows_stay_read_only_on_actual_server(self):
        async with native_server() as (server, client, address):
            for target in TARGETS[2:]:
                value = await server.read_property(target.oid, target.prop)
                with self.assertRaises(BacnetProtocolError) as raised:
                    await client.write_property(address, target.oid, target.prop, value)
                self.assertEqual(raised.exception.error_class, ErrorClass.PROPERTY.to_raw())
                self.assertEqual(raised.exception.error_code, ErrorCode.WRITE_ACCESS_DENIED.to_raw())
