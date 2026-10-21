"""Typed reads of the remaining single constructed values (#1499): Access
Point Access_Event_Time and Access_Event_Authentication_Factor, Device
Last_Restore_Time, Credential Data Input Present_Value, Load Control's shed
levels, Structured View Subordinate_List and Event Enrollment
Event_Parameters, through every read route, with exact write-back."""
import copy
import pickle
import unittest

from rusty_bacnet import (BACnetClient, BACnetServer, BACnetTimeStamp, ObjectIdentifier,
                          ObjectType, PropertyIdentifier, PropertyValue)
from typed_reference_peer import ReferencePeer, Target

P = PropertyIdentifier
O = ObjectType
DEVICE = 9599
ROUTES = ("rp", "rpm", "device_rp", "device_rpm", "batch_rp", "batch_rpm",
          "endpoint_rp", "endpoint_rpm")

# Object words and property numbers are independently authored wire coordinates.
POINT = ObjectIdentifier(O.ACCESS_POINT, 1)
ACCESS_EVENT_TIME = Target(POINT, P.ACCESS_EVENT_TIME, (33 << 22) | 1, 250)
ACCESS_EVENT_FACTOR = Target(POINT, P.ACCESS_EVENT_AUTHENTICATION_FACTOR, (33 << 22) | 1, 248)
LAST_RESTORE_TIME = Target(ObjectIdentifier(O.DEVICE, 1), P.LAST_RESTORE_TIME, (8 << 22) | 1, 157)
READER = ObjectIdentifier(O.CREDENTIAL_DATA_INPUT, 1)
READER_VALUE = Target(READER, P.PRESENT_VALUE, (37 << 22) | 1, 85)
LOAD = ObjectIdentifier(O.LOAD_CONTROL, 1)
SHED_LEVELS = [Target(LOAD, P.REQUESTED_SHED_LEVEL, (28 << 22) | 1, 218),
               Target(LOAD, P.EXPECTED_SHED_LEVEL, (28 << 22) | 1, 214),
               Target(LOAD, P.ACTUAL_SHED_LEVEL, (28 << 22) | 1, 212)]
VIEW = ObjectIdentifier(O.STRUCTURED_VIEW, 1)
SUBORDINATES = Target(VIEW, P.SUBORDINATE_LIST, (29 << 22) | 1, 211)
ENROLLMENT = ObjectIdentifier(O.EVENT_ENROLLMENT, 1)
PARAMETERS = Target(ENROLLMENT, P.EVENT_PARAMETERS, (9 << 22) | 1, 83)

UNSPECIFIED = (255, 255, 255, 255)
TIMESTAMPS = [
    (bytes.fromhex("1907"), BACnetTimeStamp.sequence_number(7)),
    (bytes.fromhex("0c0c22380b"), BACnetTimeStamp.time(12, 34, 56, 11)),
    (bytes.fromhex("2ea47e0a1301b40c22380b2f"),
     BACnetTimeStamp.date_time((2026, 10, 19, 1), (12, 34, 56, 11))),
    # The unset form: every date and time field unspecified.
    (bytes.fromhex("2ea4ffffffffb4ffffffff2f"), BACnetTimeStamp.date_time(UNSPECIFIED, UNSPECIFIED)),
]
FACTORS = [
    (bytes.fromhex("090619022a1234"), (6, 2, b"\x12\x34")),
    # The UNDEFINED factor, what an Access Point or reader starts with.
    (bytes.fromhex("0900190028"), (0, 0, b"")),
]
SHEDS = [
    (bytes.fromhex("0932"), {"kind": "percent", "value": 50}),
    (bytes.fromhex("1903"), {"kind": "level", "value": 3}),
    (bytes.fromhex("2c40200000"), {"kind": "amount", "value": 2.5}),
    (bytes.fromhex("1900"), {"kind": "level", "value": 0}),
]
AV7 = ObjectIdentifier(O.ANALOG_VALUE, 7)
REMOTE = ObjectIdentifier(O.DEVICE, 99)
SUBORDINATE_LISTS = [
    (bytes.fromhex("1c00800007"), [AV7]),
    (bytes.fromhex("1c008000070c020000631c00800007"), [AV7, (REMOTE, AV7)]),
    (b"", []),
]
SETPOINT = {"object_identifier": AV7, "property_identifier": P.PRESENT_VALUE,
            "property_array_index": None, "device_identifier": None}
EVENT_PARAMETERS = [
    (bytes.fromhex("5e091e1c41a000002c422000003c3f8000005f"),
     {"kind": "out_of_range", "time_delay": 30, "low_limit": 20.0, "high_limit": 40.0,
      "deadband": 1.0}),
    (bytes.fromhex("4e090a1e0c0080000719551f2c3f8000003c400000004c3f0000004f"),
     {"kind": "floating_limit", "time_delay": 10, "setpoint_reference": SETPOINT,
      "low_diff_limit": 1.0, "high_diff_limit": 2.0, "deadband": 0.5}),
    (bytes.fromhex("2e09001e1c3f0000001f2f"),
     {"kind": "change_of_value", "time_delay": 0, "bitmask": None,
      "referenced_property_increment": 0.5}),
    (bytes.fromhex("1e09051e090119011f1f"),
     {"kind": "change_of_state", "time_delay": 5, "list_of_values": [(0, True), (1, 1)]}),
    (bytes.fromhex("0e09021a05a02e8205a08205402f0f"),
     {"kind": "change_of_bitstring", "time_delay": 2,
      "bitmask": {"unused_bits": 5, "data": b"\xa0"},
      "list_of_values": [{"unused_bits": 5, "data": b"\xa0"},
                         {"unused_bits": 5, "data": b"\x40"}]}),
    (bytes.fromhex("9e0a022b19092e21052f9f"),
     {"kind": "extended", "vendor_id": 555, "extended_event_type": 9,
      "parameters": b"\x21\x05"}),
    # Command-failure, an algorithm the stack keeps as its encoded contents.
    (bytes.fromhex("3e09051e0c0080000719551f3f"),
     {"kind": "other", "tag": 3, "data": bytes.fromhex("09051e0c0080000719551f")}),
    # The none alternative, context tag 20 with no contents.
    (bytes.fromhex("f814"), {"kind": "none"}),
]


def vectors(target):
    if target in (ACCESS_EVENT_TIME, LAST_RESTORE_TIME):
        return "timestamp", TIMESTAMPS
    if target in (ACCESS_EVENT_FACTOR, READER_VALUE):
        return "authentication_factor", FACTORS
    if target in SHED_LEVELS:
        return "shed_level", SHEDS
    if target == SUBORDINATES:
        return "list", SUBORDINATE_LISTS
    return "event_parameter", EVENT_PARAMETERS


SINGLE_TARGETS = [ACCESS_EVENT_TIME, ACCESS_EVENT_FACTOR, LAST_RESTORE_TIME, READER_VALUE,
                  *SHED_LEVELS, PARAMETERS]


class PeerSingleReadTests(unittest.IsolatedAsyncioTestCase):
    async def test_each_property_reads_typed_on_every_route(self):
        for target in [*SINGLE_TARGETS, SUBORDINATES]:
            tag, examples = vectors(target)
            chosen = [examples[1]] if target == SUBORDINATES else [examples[0]]
            if target == PARAMETERS:
                # Every Event_Parameters form, the extended and other ones included.
                chosen = examples
            for raw, expected in chosen:
                for route in ROUTES:
                    with self.subTest(prop=target.prop, route=route, raw=raw.hex()):
                        value = await ReferencePeer(self, target).read(raw, route=route)
                        self.assertEqual((value.tag, value.value), (tag, expected))
                        self.assertEqual(value.octets, raw)

    async def test_every_form_writes_back_its_own_octets_and_survives_pickling(self):
        for target in [*SINGLE_TARGETS, SUBORDINATES]:
            tag, examples = vectors(target)
            for raw, expected in examples:
                with self.subTest(prop=target.prop, raw=raw.hex()):
                    value = await ReferencePeer(self, target).read(raw, echo=True)
                    self.assertEqual((value.tag, value.value), (tag, expected))
                    self.assertEqual(pickle.loads(pickle.dumps(value)), value)
                    self.assertEqual(copy.deepcopy(value).value, expected)

    async def test_subordinate_list_elements_read_by_index(self):
        raw = bytes.fromhex("0c02000063") + bytes.fromhex("1c00800007")
        value = await ReferencePeer(self, SUBORDINATES).read(raw, 2, echo=True)
        self.assertEqual((value.tag, value.value), ("device_object_reference", (REMOTE, AV7)))
        size = await ReferencePeer(self, SUBORDINATES).read(b"\x21\x02", 0)
        self.assertEqual((size.tag, size.value), ("unsigned", 2))

    def test_octets_of_primitive_raw_and_list_values(self):
        self.assertEqual(PropertyValue.real(1.5).octets, bytes.fromhex("443fc00000"))
        self.assertEqual(PropertyValue.unsigned(2).octets, bytes.fromhex("2102"))
        self.assertEqual(PropertyValue.application_data(b"\x19\x02").octets, b"\x19\x02")
        self.assertEqual(PropertyValue.list([]).octets, b"")
        self.assertEqual(PropertyValue.list([PropertyValue.null(), PropertyValue.boolean(True)]).octets,
                         bytes.fromhex("0011"))

    async def test_values_that_are_not_one_element_stay_generic(self):
        for target in SINGLE_TARGETS:
            raw = vectors(target)[1][0][0]
            for malformed in (raw + raw, raw + b"\x21\x07"):
                with self.subTest(prop=target.prop, raw=malformed.hex()):
                    value = await ReferencePeer(self, target).read(malformed)
                    self.assertEqual((value.tag, value.value), ("application_data", malformed))
            value = await ReferencePeer(self, target).read(b"\x21\x07")
            self.assertEqual((value.tag, value.value), ("unsigned", 7))
        # An application octet string, the legacy form older servers gave an
        # unset enrollment, is not the CHOICE, so it keeps its generic shape.
        value = await ReferencePeer(self, PARAMETERS).read(b"\x60")
        self.assertEqual((value.tag, value.value), ("octet_string", b""))


class NativeSingleReadTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.server = BACnetServer(DEVICE, interface="127.0.0.1", port=0,
                                   broadcast_address="127.0.0.1")
        self.server.add_access_point(1, "Point")
        self.server.add_credential_data_input(1, "Reader", supported_formats=[(6, 2)])
        self.server.add_load_control(1, "Load")
        self.server.add_structured_view(1, "View")
        self.server.add_event_enrollment(1, "Enrollment")
        await self.server.start()
        self.client = BACnetClient(interface="127.0.0.1", port=0)
        await self.client.__aenter__()
        self.address = await self.server.local_address()

    async def asyncTearDown(self):
        await self.client.__aexit__(None, None, None)
        await self.server.stop()

    async def reads(self, oid, prop, tag, expected):
        local = await self.server.read_property(oid, prop)
        direct = await self.client.read_property(self.address, oid, prop)
        rpm = await self.client.read_property_multiple(self.address, [(oid, [(prop, None)])])
        row = rpm[0]["results"][0]
        self.assertIsNone(row["error"])
        for value in (local, direct, row["value"]):
            self.assertEqual((value.tag, value.value), (tag, expected))
            self.assertEqual(value, local)
        return local

    async def test_access_point_event_time_and_factor_unset_then_reported(self):
        await self.reads(POINT, P.ACCESS_EVENT_TIME, "timestamp",
                         BACnetTimeStamp.date_time(UNSPECIFIED, UNSPECIFIED))
        unset = await self.reads(POINT, P.ACCESS_EVENT_AUTHENTICATION_FACTOR,
                                 "authentication_factor", (0, 0, b""))
        self.assertEqual(unset.octets, FACTORS[1][0])
        stamp = BACnetTimeStamp.sequence_number(4)
        await self.server.report_access_event_local(POINT, 1, 7, time=stamp,
                                                    authentication_factor=(6, 2, b"\x12\x34"))
        await self.reads(POINT, P.ACCESS_EVENT_TIME, "timestamp", stamp)
        factor = await self.reads(POINT, P.ACCESS_EVENT_AUTHENTICATION_FACTOR,
                                  "authentication_factor", (6, 2, b"\x12\x34"))
        self.assertEqual(factor.octets, FACTORS[0][0])
        # The read form goes straight back into the report route.
        await self.server.report_access_event_local(POINT, 1, 8, authentication_factor=unset.value)
        await self.reads(POINT, P.ACCESS_EVENT_AUTHENTICATION_FACTOR, "authentication_factor",
                         (0, 0, b""))

    async def test_reader_present_value_round_trips_through_report_credential_read(self):
        unset = await self.reads(READER, P.PRESENT_VALUE, "authentication_factor", (0, 0, b""))
        await self.server.report_credential_read_local(READER, (6, 2, b"\x12\x34"))
        read = await self.reads(READER, P.PRESENT_VALUE, "authentication_factor",
                                (6, 2, b"\x12\x34"))
        await self.server.report_credential_read_local(READER, unset.value)
        await self.reads(READER, P.PRESENT_VALUE, "authentication_factor", (0, 0, b""))
        await self.server.report_credential_read_local(READER, read.value)
        again = await self.reads(READER, P.PRESENT_VALUE, "authentication_factor", read.value)
        self.assertEqual(again, read)

    async def test_shed_levels_read_typed_and_a_written_level_reads_back(self):
        for target in SHED_LEVELS:
            await self.reads(LOAD, target.prop, "shed_level", {"kind": "level", "value": 0})
        level = PropertyValue.application_data(bytes.fromhex("1902"))
        await self.client.write_property(self.address, LOAD, P.REQUESTED_SHED_LEVEL, level)
        read = await self.reads(LOAD, P.REQUESTED_SHED_LEVEL, "shed_level",
                                {"kind": "level", "value": 2})
        await self.client.write_property(self.address, LOAD, P.REQUESTED_SHED_LEVEL, read)
        self.assertEqual(await self.server.read_property(LOAD, P.REQUESTED_SHED_LEVEL), read)
        self.assertEqual(read.octets, bytes.fromhex("1902"))

    async def test_empty_subordinate_list_and_event_parameters_set_locally(self):
        await self.reads(VIEW, P.SUBORDINATE_LIST, "list", [])
        # Unset parameters serve the CHOICE's none alternative (#1868).
        unset = await self.reads(ENROLLMENT, P.EVENT_PARAMETERS, "event_parameter",
                                 {"kind": "none"})
        self.assertEqual(unset.octets, bytes.fromhex("f814"))
        raw, expected = EVENT_PARAMETERS[0]
        await self.server.write_property_local(ENROLLMENT, P.EVENT_PARAMETERS,
                                               PropertyValue.application_data(raw),
                                               source_object=None)
        read = await self.reads(ENROLLMENT, P.EVENT_PARAMETERS, "event_parameter", expected)
        self.assertEqual(read.octets, raw)
        await self.client.write_property(self.address, ENROLLMENT, P.EVENT_PARAMETERS, read)
        self.assertEqual(await self.server.read_property(ENROLLMENT, P.EVENT_PARAMETERS), read)


if __name__ == "__main__":
    unittest.main()
