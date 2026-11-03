"""Endpoint object builders take the server builders' profile and Value
keywords (#1867): endpoint-built objects serve Tags, Profile_Location,
Profile_Name and Value_Source, and refuse the keywords the server refuses."""
import unittest

from rusty_bacnet import (BACnetClient, BacnetProtocolError, BipEndpoint, ErrorCode,
                          MstpEndpoint, ObjectIdentifier, ObjectType, PropertyIdentifier,
                          PropertyValue)

P = PropertyIdentifier
O = ObjectType
TAGS = [{"name": "zone", "value": PropertyValue.unsigned(3)},
        {"name": "exhaust", "value": None},
        {"name": "label", "value": PropertyValue.character_string("north")}]
PROFILE = {"tags": TAGS, "profile_name": "555-room",
           "profile_location": "https://example.com/room.json"}


def bip_endpoint():
    return BipEndpoint(device_instance=1867, interface="127.0.0.1", port=0,
                       broadcast_address="127.255.255.255")


class EndpointObjectProfileTests(unittest.IsolatedAsyncioTestCase):
    async def test_endpoint_objects_serve_profile_rows_and_value_source(self):
        endpoint = bip_endpoint()
        endpoint.add_analog_input(1, "AI", **PROFILE)
        endpoint.add_analog_value(1, "AV", present_value_access="writable",
                                  value_source_tracking=True, **PROFILE)
        endpoint.add_binary_input(1, "BI", **PROFILE)
        endpoint.add_binary_value(1, "BV", present_value_access="read_only",
                                  value_source_tracking=True, **PROFILE)
        endpoint.add_analog_value(2, "Plain AV")
        endpoint.add_binary_value(2, "Plain BV", present_value_access="writable")
        tagged = [ObjectIdentifier(O.ANALOG_INPUT, 1), ObjectIdentifier(O.ANALOG_VALUE, 1),
                  ObjectIdentifier(O.BINARY_INPUT, 1), ObjectIdentifier(O.BINARY_VALUE, 1)]
        tracked = tagged[1::2]
        # A commandable object serves Value_Source through Priority_Array;
        # an untracked writable one serves none.
        plain = [(ObjectIdentifier(O.ANALOG_VALUE, 2), (P.TAGS, P.PROFILE_NAME)),
                 (ObjectIdentifier(O.BINARY_VALUE, 2), (P.VALUE_SOURCE, P.TAGS))]
        async with endpoint, BACnetClient(interface="127.0.0.1", port=0) as client:
            address = await endpoint.local_address()
            for oid in tagged:
                with self.subTest(oid=oid):
                    tags = await client.read_property(address, oid, P.TAGS)
                    self.assertEqual((tags.tag, tags.value), ("list", TAGS))
                    name = await client.read_property(address, oid, P.PROFILE_NAME)
                    self.assertEqual(name.value, "555-room")
                    location = await client.read_property(address, oid, P.PROFILE_LOCATION)
                    self.assertEqual(location.value, "https://example.com/room.json")
            for oid in tracked:
                with self.subTest(oid=oid):
                    source = await client.read_property(address, oid, P.VALUE_SOURCE)
                    self.assertEqual((source.tag, source.value), ("value_source", None))
            for oid, absent in plain:
                for prop in absent:
                    with self.subTest(oid=oid, prop=prop):
                        with self.assertRaises(BacnetProtocolError) as raised:
                            await client.read_property(address, oid, prop)
                        self.assertEqual(raised.exception.error_code,
                                         ErrorCode.UNKNOWN_PROPERTY.to_raw())

    def test_builders_refuse_what_the_server_builders_refuse(self):
        endpoints = [bip_endpoint(),
                     MstpEndpoint(device_instance=1867, serial_port="/tmp/nonexistent")]
        for endpoint in endpoints:
            with self.subTest(endpoint=type(endpoint).__name__):
                with self.assertRaises(ValueError):
                    endpoint.add_analog_value(1, "AV", value_source_tracking=True)
                with self.assertRaises(ValueError):
                    endpoint.add_binary_value(1, "BV", present_value_access="commandable",
                                              value_source_tracking=True)
                with self.assertRaises(ValueError):
                    endpoint.add_analog_value(1, "AV", present_value_access="sometimes")
                with self.assertRaises(BacnetProtocolError):
                    endpoint.add_binary_input(1, "BI", profile_name="room")
                with self.assertRaises(TypeError):
                    endpoint.add_analog_input(1, "AI", tags="zone")
                with self.assertRaises(TypeError):
                    endpoint.add_analog_input(1, "AI", 62, 0.0, None)
                # Nothing refused was kept.
                self.assertEqual(endpoint._pending_registration_count(), 0)


if __name__ == "__main__":
    unittest.main()
