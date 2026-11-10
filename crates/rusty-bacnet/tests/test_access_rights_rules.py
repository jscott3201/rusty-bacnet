"""Installed-artifact tests for add_access_rights' keyword arguments.

positive_access_rules=... and negative_access_rules=... set Access Rights'
Positive_Access_Rules and Negative_Access_Rules (#1316), and enable=... its
Enable row, property 133 (#1332). Peers can write all three (#1330), and
storage_path=... keeps what they write across a restart (#1392; see
test_access_rights_persistence.py). accompaniment=... serves the optional
Accompaniment row (#1393; see test_access_rights_accompaniment.py).
"""

from __future__ import annotations

import ast
import asyncio
import inspect
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetProtocolError,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

SCHEDULE = ObjectIdentifier(ObjectType.SCHEDULE, 1)
LOBBY = ObjectIdentifier(ObjectType.ACCESS_POINT, 2)
REMOTE_DEVICE = ObjectIdentifier(ObjectType.DEVICE, 99)
REMOTE_ZONE = ObjectIdentifier(ObjectType.ACCESS_ZONE, 3)

# A BACnetAccessRule: time-range specifier [0], time range [1] (a
# BACnetDeviceObjectPropertyReference), location specifier [2], location [3]
# (a BACnetDeviceObjectReference) and enable [4]; SPECIFIED is 0, ALWAYS and
# ALL are 1.
BUSINESS_HOURS = bytes(
    [0x09, 0x00, 0x1E, 0x0C, 0x04, 0x40, 0x00, 0x01, 0x19, 0x55, 0x1F]
    + [0x29, 0x00, 0x3E, 0x1C, 0x08, 0x40, 0x00, 0x02, 0x3F, 0x49, 0x01]
)
ANYWHERE_OFF = bytes([0x09, 0x01, 0x29, 0x01, 0x49, 0x00])
REMOTE_LOCKDOWN = bytes(
    [0x09, 0x01, 0x29, 0x00, 0x3E, 0x0C, 0x02, 0x00, 0x00, 0x63]
    + [0x1C, 0x09, 0x00, 0x00, 0x03, 0x3F, 0x49, 0x01]
)

# The rule an index-0 write appends: SPECIFIED with an unspecified Schedule
# Present_Value, SPECIFIED with an unspecified Access Point, disabled.
GROWN = bytes(
    [0x09, 0x00, 0x1E, 0x0C, 0x04, 0x7F, 0xFF, 0xFF, 0x19, 0x55, 0x1F]
    + [0x29, 0x00, 0x3E, 0x1C, 0x08, 0x7F, 0xFF, 0xFF, 0x3F, 0x49, 0x00]
)

# The same rules as a read gives them back (#1344): every key present, None
# standing for ALWAYS and ALL.
def reference(oid: ObjectIdentifier) -> dict:
    return {
        "object_identifier": oid,
        "property_identifier": PropertyIdentifier.PRESENT_VALUE,
        "property_array_index": None,
        "device_identifier": None,
    }


ANYWHERE_OFF_RULE = {"enable": False, "time_range": None, "location": None}
REMOTE_LOCKDOWN_RULE = {
    "enable": True,
    "time_range": None,
    "location": (ObjectIdentifier(ObjectType.DEVICE, 99), ObjectIdentifier(ObjectType.ACCESS_ZONE, 3)),
}
GROWN_RULE = {
    "enable": False,
    "time_range": reference(ObjectIdentifier(ObjectType.SCHEDULE, 4194303)),
    "location": ObjectIdentifier(ObjectType.ACCESS_POINT, 4194303),
}
BUSINESS_HOURS_RULE = {
    "enable": True,
    "time_range": reference(ObjectIdentifier(ObjectType.SCHEDULE, 1)),
    "location": ObjectIdentifier(ObjectType.ACCESS_POINT, 2),
}

RULE_KEYWORDS = ["positive_access_rules", "negative_access_rules"]
KEYWORDS = [*RULE_KEYWORDS, "enable", "accompaniment", "storage_path"]


def installed_stub_method(name: str) -> ast.FunctionDef:
    stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    tree = ast.parse(stub_path.read_text(encoding="utf-8"), filename=str(stub_path))
    server = next(
        node
        for node in tree.body
        if isinstance(node, ast.ClassDef) and node.name == "BACnetServer"
    )
    return next(
        node
        for node in server.body
        if isinstance(node, ast.FunctionDef) and node.name == name
    )


def make_server() -> BACnetServer:
    return BACnetServer(
        device_instance=503_316,
        device_name="Access Rights Rules Test",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )


def business_hours() -> dict:
    return {
        "time_range": {
            "object_identifier": SCHEDULE,
            "property_identifier": PropertyIdentifier.PRESENT_VALUE,
        },
        "location": LOBBY,
        "enable": True,
    }


class AccessRightsStubContractTests(unittest.TestCase):
    def test_runtime_and_stub_expose_the_keywords(self) -> None:
        parameters = inspect.signature(BACnetServer.add_access_rights).parameters
        self.assertEqual(list(parameters), ["self", "instance", "name", *KEYWORDS])
        defaults = {
            "positive_access_rules": None,
            "negative_access_rules": None,
            "enable": True,
            "accompaniment": None,
            "storage_path": None,
        }
        for keyword in KEYWORDS:
            with self.subTest(keyword=keyword):
                self.assertIs(parameters[keyword].kind, inspect.Parameter.KEYWORD_ONLY)
                self.assertIs(parameters[keyword].default, defaults[keyword])
        method = installed_stub_method("add_access_rights")
        self.assertEqual(
            [argument.arg for argument in method.args.args], ["self", "instance", "name"]
        )
        self.assertEqual([argument.arg for argument in method.args.kwonlyargs], KEYWORDS)
        for argument, default in zip(method.args.kwonlyargs, method.args.kw_defaults):
            self.assertIsInstance(default, ast.Constant)
            self.assertIs(default.value, defaults[argument.arg])


class AccessRightsRulesTests(unittest.TestCase):
    def test_rules_reach_both_arrays_and_read_back(self) -> None:
        asyncio.run(self._rules_read_back())

    async def _rules_read_back(self) -> None:
        server = make_server()
        server.add_access_rights(
            1,
            "Employee Access",
            positive_access_rules=[business_hours(), {"enable": False}],
            negative_access_rules=[
                {"location": (REMOTE_DEVICE, REMOTE_ZONE), "enable": True}
            ],
        )
        server.add_access_rights(2, "No Rules")
        await server.start()
        try:
            address = await server.local_address()
            async with BACnetClient(
                interface="127.0.0.1", port=0, apdu_timeout_ms=2000
            ) as client:
                await self._read_back(server, client, address)
        finally:
            await server.stop()

    async def _read_back(
        self, server: BACnetServer, client: BACnetClient, address: str
    ) -> None:
        rights = ObjectIdentifier(ObjectType.ACCESS_RIGHTS, 1)
        positive = PropertyIdentifier.POSITIVE_ACCESS_RULES
        negative = PropertyIdentifier.NEGATIVE_ACCESS_RULES

        async def read(oid: ObjectIdentifier, prop: PropertyIdentifier, index=None):
            value = await client.read_property(address, oid, prop, index)
            # A local read sees the octets a peer does.
            self.assertEqual(await server.read_property(oid, prop, index), value)
            return value.value

        self.assertEqual(await read(rights, positive, 0), 2)
        self.assertEqual(await read(rights, positive, 1), BUSINESS_HOURS_RULE)
        self.assertEqual(await read(rights, positive, 2), ANYWHERE_OFF_RULE)
        # A whole read is every rule, in order.
        self.assertEqual(await read(rights, positive), [BUSINESS_HOURS_RULE, ANYWHERE_OFF_RULE])
        self.assertEqual(await read(rights, negative, 0), 1)
        self.assertEqual(await read(rights, negative, 1), REMOTE_LOCKDOWN_RULE)
        with self.assertRaises(BacnetProtocolError) as raised:
            await client.read_property(address, rights, negative, 2)
        self.assertEqual(raised.exception.error_code, ErrorCode.INVALID_ARRAY_INDEX.to_raw())

        bare = ObjectIdentifier(ObjectType.ACCESS_RIGHTS, 2)
        for prop in (positive, negative):
            self.assertEqual(await read(bare, prop, 0), 0)
        # Enable defaults to TRUE.
        self.assertIs(await read(rights, PropertyIdentifier.ENABLE), True)

    def test_network_writes_reach_the_arrays_and_enable(self) -> None:
        asyncio.run(self._network_writes())

    async def _network_writes(self) -> None:
        server = make_server()
        server.add_access_rights(1, "Employee Access", enable=False)
        await server.start()
        try:
            address = await server.local_address()
            async with BACnetClient(
                interface="127.0.0.1", port=0, apdu_timeout_ms=2000
            ) as client:
                await self._write_back(server, client, address)
        finally:
            await server.stop()

    async def _write_back(
        self, server: BACnetServer, client: BACnetClient, address: str
    ) -> None:
        rights = ObjectIdentifier(ObjectType.ACCESS_RIGHTS, 1)
        positive = PropertyIdentifier.POSITIVE_ACCESS_RULES
        negative = PropertyIdentifier.NEGATIVE_ACCESS_RULES
        enable = PropertyIdentifier.ENABLE

        async def read(prop: PropertyIdentifier, index=None):
            value = await client.read_property(address, rights, prop, index)
            self.assertEqual(await server.read_property(rights, prop, index), value)
            return value.value

        async def write(prop: PropertyIdentifier, value: PropertyValue, index=None):
            await client.write_property(address, rights, prop, value, array_index=index)

        self.assertIs(await read(enable), False)
        await write(enable, PropertyValue.boolean(True))
        self.assertIs(await read(enable), True)

        # A whole array, one element, then a resize at index 0.
        await write(positive, PropertyValue.application_data(BUSINESS_HOURS + ANYWHERE_OFF))
        self.assertEqual(await read(positive), [BUSINESS_HOURS_RULE, ANYWHERE_OFF_RULE])
        await write(positive, PropertyValue.application_data(REMOTE_LOCKDOWN), 2)
        self.assertEqual(await read(positive, 2), REMOTE_LOCKDOWN_RULE)
        await write(negative, PropertyValue.unsigned(2), 0)
        self.assertEqual(await read(negative, 0), 2)
        self.assertEqual(await read(negative), [GROWN_RULE, GROWN_RULE])
        await write(positive, PropertyValue.unsigned(1), 0)
        self.assertEqual(await read(positive), [BUSINESS_HOURS_RULE])

        # WritePropertyMultiple writes both arrays and Enable in one request.
        await client.write_property_multiple(
            address,
            [
                (
                    rights,
                    [
                        (negative, PropertyValue.application_data(ANYWHERE_OFF), None, 1),
                        (positive, PropertyValue.application_data(REMOTE_LOCKDOWN), None, None),
                        (enable, PropertyValue.boolean(False), None, None),
                    ],
                )
            ],
        )
        self.assertEqual(await read(negative), [ANYWHERE_OFF_RULE, GROWN_RULE])
        self.assertEqual(await read(positive), [REMOTE_LOCKDOWN_RULE])
        self.assertIs(await read(enable), False)

        # A rule whose location is an Access Door is refused, and the array
        # keeps its rules.
        door_rule = bytes(
            [0x09, 0x01, 0x29, 0x00, 0x3E, 0x1C, 0x07, 0x80, 0x00, 0x04, 0x3F, 0x49, 0x01]
        )
        with self.assertRaises(BacnetProtocolError) as raised:
            await write(positive, PropertyValue.application_data(door_rule), 1)
        self.assertEqual(raised.exception.error_code, ErrorCode.VALUE_OUT_OF_RANGE.to_raw())
        self.assertEqual(await read(positive), [REMOTE_LOCKDOWN_RULE])

    def test_ill_formed_rules_register_nothing(self) -> None:
        asyncio.run(self._ill_formed_rules())

    async def _ill_formed_rules(self) -> None:
        server = make_server()
        not_a_device = ObjectIdentifier(ObjectType.ANALOG_VALUE, 99)
        door = ObjectIdentifier(ObjectType.ACCESS_DOOR, 4)
        # A location that is neither an Access Point nor an Access Zone is the
        # object's refusal.
        with self.assertRaises(BacnetProtocolError) as raised:
            server.add_access_rights(
                3, "Wrong", negative_access_rules=[{"location": door, "enable": True}]
            )
        self.assertEqual(raised.exception.error_code, ErrorCode.VALUE_OUT_OF_RANGE.to_raw())
        # A device member that isn't a Device (#1285), or an unknown or
        # missing key, is a ValueError.
        time_range = dict(business_hours()["time_range"], device_identifier=not_a_device)
        for rule in (
            {"location": (not_a_device, LOBBY), "enable": True},
            {"time_range": time_range, "enable": True},
            {"enable": True, "priority": 1},
            {"location": LOBBY},
        ):
            with self.subTest(rule=rule), self.assertRaises(ValueError):
                server.add_access_rights(4, "Wrong", positive_access_rules=[rule])
        # Wrong shapes and types are TypeErrors.
        for rules in (
            business_hours(),
            [LOBBY],
            [{"enable": 1}],
            [{"location": "lobby", "enable": True}],
        ):
            with self.subTest(rules=rules), self.assertRaises(TypeError):
                server.add_access_rights(5, "Wrong", positive_access_rules=rules)
        # enable takes a bool.
        with self.assertRaises(TypeError):
            server.add_access_rights(6, "Wrong", enable="no")
        await server.start()
        try:
            for instance in (3, 4, 5, 6):
                with self.assertRaises(BacnetProtocolError) as raised:
                    await server.read_property(
                        ObjectIdentifier(ObjectType.ACCESS_RIGHTS, instance),
                        PropertyIdentifier.OBJECT_NAME,
                    )
                self.assertEqual(
                    raised.exception.error_code, ErrorCode.UNKNOWN_OBJECT.to_raw()
                )
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
