"""Lighting trims, colour links, colour objects, Value_Source tracking and
profile rows provisioned from Python before start (#1554, #1552, #1553)."""
import unittest
from contextlib import asynccontextmanager

from rusty_bacnet import (BACnetClient, BACnetServer, BacnetProtocolError, ErrorClass,
                          ErrorCode, ObjectIdentifier, ObjectType, PropertyIdentifier,
                          PropertyValue)

P = PropertyIdentifier
O = ObjectType
DEVICE = 9554
LIGHT = ObjectIdentifier(O.LIGHTING_OUTPUT, 1)
BINARY_LIGHT = ObjectIdentifier(O.BINARY_LIGHTING_OUTPUT, 1)
COLOR = ObjectIdentifier(O.COLOR, 1)
WARM = ObjectIdentifier(O.COLOR_TEMPERATURE, 1)
NO_COLOR = ObjectIdentifier(O.COLOR, 4194303)
TAGS = [{"name": "zone", "value": PropertyValue.unsigned(3)},
        {"name": "exhaust", "value": None},
        {"name": "label", "value": PropertyValue.character_string("north")}]


def new_server():
    return BACnetServer(DEVICE, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")


@asynccontextmanager
async def running(server):
    await server.start()
    try:
        yield server
    finally:
        await server.stop()


class LightingAndColorTests(unittest.IsolatedAsyncioTestCase):
    async def assert_unknown(self, server, oid, prop):
        with self.assertRaises(BacnetProtocolError) as raised:
            await server.read_property(oid, prop)
        self.assertEqual(raised.exception.error_code, ErrorCode.UNKNOWN_PROPERTY.to_raw())

    async def test_trims_serve_their_rows_and_none_leaves_them_out(self):
        server = new_server()
        server.add_lighting_output(1, "Trimmed", high_end_trim=90.0, low_end_trim=10.0,
                                   trim_fade_time=2000)
        server.add_lighting_output(2, "Plain")
        async with running(server):
            for prop, tag, expected in [(P.HIGH_END_TRIM, "real", 90.0),
                                        (P.LOW_END_TRIM, "real", 10.0),
                                        (P.TRIM_FADE_TIME, "unsigned", 2000)]:
                value = await server.read_property(LIGHT, prop)
                self.assertEqual((value.tag, value.value), (tag, expected))
                await self.assert_unknown(server, ObjectIdentifier(O.LIGHTING_OUTPUT, 2), prop)

    def test_out_of_range_trims_are_refused_before_registration(self):
        server = new_server()
        for keywords in ({"high_end_trim": 0.5}, {"low_end_trim": 100.5},
                         {"trim_fade_time": 86_400_001}):
            with self.subTest(**keywords), self.assertRaises(BacnetProtocolError) as raised:
                server.add_lighting_output(1, "Bad", **keywords)
            self.assertEqual(raised.exception.error_code, ErrorCode.VALUE_OUT_OF_RANGE.to_raw())

    async def test_color_links_serve_references_and_lighting_color_follows_them(self):
        server = new_server()
        # The start drives each colour object to its default (#1526).
        server.add_color(1, "Color", default_color=(0.25, 0.5))
        server.add_color_temperature(1, "Warm", default_color_temperature=2700,
                                     min_pres_value=2000, max_pres_value=6500)
        server.add_lighting_output(1, "Light", color_reference=COLOR,
                                   override_color_reference=WARM)
        server.add_binary_lighting_output(1, "Switch", color_reference=WARM)
        server.add_binary_lighting_output(2, "Unlinked")
        server.add_binary_lighting_output(3, "None", color_reference=NO_COLOR)
        async with running(server):
            self.assertEqual((await server.read_property(LIGHT, P.COLOR_REFERENCE)).value, COLOR)
            self.assertIs((await server.read_property(LIGHT, P.COLOR_OVERRIDE)).value, False)
            self.assertEqual((await server.read_property(LIGHT, P.OVERRIDE_COLOR_REFERENCE)).value,
                             WARM)
            await self.assert_unknown(server, BINARY_LIGHT, P.COLOR_OVERRIDE)
            await self.assert_unknown(server, ObjectIdentifier(O.BINARY_LIGHTING_OUTPUT, 2),
                                      P.COLOR_REFERENCE)

            source, (x, y) = await server.lighting_color(LIGHT)
            self.assertEqual(source, COLOR)
            self.assertAlmostEqual(x, 0.25, places=6)
            self.assertAlmostEqual(y, 0.5, places=6)
            self.assertEqual(await server.lighting_color(BINARY_LIGHT), (WARM, 2700))
            for unlinked in (2, 3):
                oid = ObjectIdentifier(O.BINARY_LIGHTING_OUTPUT, unlinked)
                self.assertIsNone(await server.lighting_color(oid))

            await server.write_property_local(LIGHT, P.COLOR_OVERRIDE, PropertyValue.boolean(True),
                                              source_object=None)
            self.assertEqual(await server.lighting_color(LIGHT), (WARM, 2700))

    async def test_color_override_false_alone_means_no_override(self):
        server = new_server()
        server.add_lighting_output(1, "Light", color_reference=COLOR, color_override=False)
        server.add_binary_lighting_output(1, "Unlinked", color_override=False)
        async with running(server):
            self.assertEqual((await server.read_property(LIGHT, P.COLOR_REFERENCE)).value, COLOR)
            await self.assert_unknown(server, LIGHT, P.COLOR_OVERRIDE)
            await self.assert_unknown(server, LIGHT, P.OVERRIDE_COLOR_REFERENCE)
            await self.assert_unknown(server, BINARY_LIGHT, P.COLOR_REFERENCE)

    async def test_an_active_override_given_at_build_time_takes_the_override_colour(self):
        server = new_server()
        server.add_color_temperature(1, "Warm", default_color_temperature=3000)
        server.add_lighting_output(1, "Light", color_reference=NO_COLOR, color_override=True,
                                   override_color_reference=WARM)
        async with running(server):
            self.assertIs((await server.read_property(LIGHT, P.COLOR_OVERRIDE)).value, True)
            self.assertEqual(await server.lighting_color(LIGHT), (WARM, 3000))

    def test_color_link_keywords_are_checked(self):
        server = new_server()
        with self.assertRaises(ValueError):
            server.add_lighting_output(1, "L", color_override=True)
        with self.assertRaises(ValueError):
            server.add_binary_lighting_output(1, "B", override_color_reference=WARM)
        with self.assertRaises(ValueError):
            server.add_lighting_output(1, "L", color_reference=COLOR, color_override=True)
        with self.assertRaises(BacnetProtocolError) as raised:
            server.add_lighting_output(1, "L", color_reference=ObjectIdentifier(O.ANALOG_VALUE, 1))
        self.assertEqual(raised.exception.error_code, ErrorCode.VALUE_OUT_OF_RANGE.to_raw())

    async def test_colour_objects_take_their_values_and_refuse_bad_ones(self):
        server = new_server()
        server.add_color(1, "Default")
        # A zero default keeps the configured present_value at start (#1526).
        server.add_color_temperature(1, "Clamped", present_value=9000, min_pres_value=2000,
                                     max_pres_value=6500, default_color_temperature=0)
        with self.assertRaises(BacnetProtocolError):
            server.add_color(2, "Bad", present_value=(1.5, 0.2))
        with self.assertRaises(BacnetProtocolError):
            server.add_color_temperature(2, "Bad", present_value=500)
        with self.assertRaises(ValueError):
            server.add_color_temperature(2, "Half", min_pres_value=2000)
        async with running(server):
            x, y = (await server.read_property(COLOR, P.PRESENT_VALUE)).value
            self.assertAlmostEqual(x, 0.3127, places=6)
            self.assertAlmostEqual(y, 0.3290, places=6)
            self.assertEqual((await server.read_property(WARM, P.PRESENT_VALUE)).value, 6500)
            self.assertEqual((await server.read_property(WARM, P.MIN_PRES_VALUE)).value, 2000)

    async def test_lighting_color_needs_a_running_server(self):
        with self.assertRaises(RuntimeError):
            await new_server().lighting_color(LIGHT)


class ValueSourceAndProfileTests(unittest.IsolatedAsyncioTestCase):
    async def test_tracked_objects_serve_value_source_and_untracked_ones_do_not(self):
        server = new_server()
        server.add_color(1, "Tracked", value_source_tracking=True)
        server.add_color(2, "Untracked")
        server.add_analog_value(1, "Writable", present_value_access="writable",
                                value_source_tracking=True)
        server.add_binary_value(1, "Read only", present_value_access="read_only",
                                value_source_tracking=True)
        server.add_multistate_value(1, "States", 3, present_value_access="writable",
                                    value_source_tracking=True)
        server.add_color_temperature(1, "Tracked warm", value_source_tracking=True)
        tracked = [(COLOR, PropertyValue.list([PropertyValue.real(0.25), PropertyValue.real(0.5)])),
                   (ObjectIdentifier(O.ANALOG_VALUE, 1), PropertyValue.real(21.5)),
                   (ObjectIdentifier(O.MULTI_STATE_VALUE, 1), PropertyValue.unsigned(2)),
                   (WARM, PropertyValue.unsigned(3000))]
        async with running(server):
            for oid, value in tracked:
                with self.subTest(oid=oid):
                    before = await server.read_property(oid, P.VALUE_SOURCE)
                    self.assertEqual((before.tag, before.value), ("value_source", None))
                    await server.write_property_local(oid, P.PRESENT_VALUE, value,
                                                      source_object=None)
                    after = await server.read_property(oid, P.VALUE_SOURCE)
                    self.assertEqual((after.tag, after.value),
                                     ("value_source", ObjectIdentifier(O.DEVICE, DEVICE)))
            # Read-only: the application, or a writer while out of service.
            binary = ObjectIdentifier(O.BINARY_VALUE, 1)
            source = await server.read_property(binary, P.VALUE_SOURCE)
            self.assertEqual((source.tag, source.value), ("value_source", None))
            await server.write_property_local(binary, P.OUT_OF_SERVICE,
                                              PropertyValue.boolean(True), source_object=None)
            await server.write_property_local(binary, P.PRESENT_VALUE,
                                              PropertyValue.enumerated(1), source_object=None)
            source = await server.read_property(binary, P.VALUE_SOURCE)
            self.assertEqual((source.tag, source.value),
                             ("value_source", ObjectIdentifier(O.DEVICE, DEVICE)))
            with self.assertRaises(BacnetProtocolError) as raised:
                await server.read_property(ObjectIdentifier(O.COLOR, 2), P.VALUE_SOURCE)
            self.assertEqual(raised.exception.error_code, ErrorCode.UNKNOWN_PROPERTY.to_raw())

    def test_value_source_tracking_needs_a_noncommandable_value(self):
        server = new_server()
        with self.assertRaises(ValueError):
            server.add_analog_value(1, "AV", value_source_tracking=True)
        with self.assertRaises(ValueError):
            server.add_binary_value(1, "BV", present_value_access="commandable",
                                    value_source_tracking=True)
        with self.assertRaises(ValueError):
            server.add_multistate_value(1, "MSV", 2, present_value_access="sometimes")

    async def test_every_builder_with_a_profile_serves_typed_tags_and_the_text_rows(self):
        server = new_server()
        builders = [
            (ObjectIdentifier(O.ANALOG_INPUT, 1), server.add_analog_input, ()),
            (ObjectIdentifier(O.ANALOG_OUTPUT, 1), server.add_analog_output, ()),
            (ObjectIdentifier(O.ANALOG_VALUE, 1), server.add_analog_value, ()),
            (ObjectIdentifier(O.BINARY_INPUT, 1), server.add_binary_input, ()),
            (ObjectIdentifier(O.BINARY_OUTPUT, 1), server.add_binary_output, ()),
            (ObjectIdentifier(O.BINARY_VALUE, 1), server.add_binary_value, ()),
            (ObjectIdentifier(O.MULTI_STATE_INPUT, 1), server.add_multistate_input, (3,)),
            (ObjectIdentifier(O.MULTI_STATE_OUTPUT, 1), server.add_multistate_output, (3,)),
            (ObjectIdentifier(O.MULTI_STATE_VALUE, 1), server.add_multistate_value, (3,)),
            (COLOR, server.add_color, ()),
            (WARM, server.add_color_temperature, ()),
            (LIGHT, server.add_lighting_output, ()),
            (BINARY_LIGHT, server.add_binary_lighting_output, ()),
        ]
        for oid, add, extra in builders:
            add(1, f"Tagged {oid}", *extra, tags=TAGS, profile_name="555-room",
                profile_location="https://example.com/room.json")
        server.add_analog_value(2, "Untagged")
        server.add_analog_value(3, "Empty", tags=[])
        async with running(server), BACnetClient(interface="127.0.0.1", port=0) as client:
            address = await server.local_address()
            for oid, _, _ in builders:
                with self.subTest(oid=oid):
                    local = await server.read_property(oid, P.TAGS)
                    remote = await client.read_property(address, oid, P.TAGS)
                    rpm = await client.read_property_multiple(address, [(oid, [(P.TAGS, None)])])
                    for value in (local, remote, rpm[0]["results"][0]["value"]):
                        self.assertEqual(value.tag, "list")
                        self.assertEqual(value.value, TAGS)
                        self.assertEqual(value, local)
                    element = await server.read_property(oid, P.TAGS, 2)
                    self.assertEqual((element.tag, element.value), ("name_value", TAGS[1]))
                    name = await server.read_property(oid, P.PROFILE_NAME)
                    self.assertEqual(name.value, "555-room")
                    location = await server.read_property(oid, P.PROFILE_LOCATION)
                    self.assertEqual(location.value, "https://example.com/room.json")
            empty = await server.read_property(ObjectIdentifier(O.ANALOG_VALUE, 3), P.TAGS)
            self.assertEqual((empty.tag, empty.value), ("list", []))
            with self.assertRaises(BacnetProtocolError):
                await server.read_property(ObjectIdentifier(O.ANALOG_VALUE, 2), P.TAGS)
            with self.assertRaises(BacnetProtocolError):
                await server.read_property(ObjectIdentifier(O.ANALOG_VALUE, 3), P.PROFILE_NAME)

    async def test_tags_read_back_build_an_object_with_the_same_bytes(self):
        first = new_server()
        first.add_analog_value(1, "Source", tags=TAGS)
        async with running(first):
            read = await first.read_property(ObjectIdentifier(O.ANALOG_VALUE, 1), P.TAGS)
        second = new_server()
        second.add_analog_value(1, "Copy", tags=read.value)
        async with running(second):
            copied = await second.read_property(ObjectIdentifier(O.ANALOG_VALUE, 1), P.TAGS)
        self.assertEqual(copied, read)
        # Independently encoded: each name in context tag 0, then its value.
        self.assertEqual(read.octets, bytes.fromhex(
            "0d05007a6f6e652103" "0d080065786861757374" "0d06006c6162656c" "7506006e6f727468"))
        self.assertEqual(copied.octets, read.octets)

    def test_profiles_the_object_refuses_raise_and_shapes_are_checked(self):
        server = new_server()
        for keywords, code in [({"profile_name": "room"}, ErrorCode.VALUE_OUT_OF_RANGE),
                               ({"profile_location": "ftp://x"}, ErrorCode.VALUE_OUT_OF_RANGE),
                               ({"tags": [{"name": "a;b"}]}, ErrorCode.VALUE_OUT_OF_RANGE),
                               ({"tags": [{"name": "n", "value": PropertyValue.list([])}]},
                                ErrorCode.INVALID_DATA_TYPE)]:
            with self.subTest(**keywords), self.assertRaises(BacnetProtocolError) as raised:
                server.add_color(1, "Bad", **keywords)
            self.assertEqual(raised.exception.error_code, code.to_raw())
        for tags, error in [("zone", TypeError), ([("zone", None)], TypeError),
                            ([{"name": 3}], TypeError), ([{"value": None}], ValueError),
                            ([{"name": "n", "colour": 1}], ValueError),
                            ([{"name": "n", "value": 3}], TypeError)]:
            with self.subTest(tags=tags), self.assertRaises(error):
                server.add_analog_input(1, "Bad", tags=tags)

    async def test_network_tags_writes_reach_a_python_provisioned_object(self):
        server = new_server()
        server.add_binary_input(1, "Writable tags", tags=TAGS[:1])
        oid = ObjectIdentifier(O.BINARY_INPUT, 1)
        async with running(server), BACnetClient(interface="127.0.0.1", port=0) as client:
            address = await server.local_address()
            full = await server.read_property(oid, P.TAGS)
            await client.write_property(address, oid, P.TAGS, PropertyValue.list([]))
            self.assertEqual((await server.read_property(oid, P.TAGS)).value, [])
            await client.write_property(address, oid, P.TAGS, full)
            self.assertEqual(await server.read_property(oid, P.TAGS), full)
            with self.assertRaises(BacnetProtocolError) as raised:
                await client.write_property(address, oid, P.PROFILE_NAME,
                                            PropertyValue.character_string("555-x"))
            self.assertEqual(raised.exception.error_class, ErrorClass.PROPERTY.to_raw())


if __name__ == "__main__":
    unittest.main()
