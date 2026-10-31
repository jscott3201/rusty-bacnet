"""Installed-artifact tests for add_channel (#1262).

add_channel registers a Channel with its List_Of_Object_Property_References,
Execution_Delay, Control_Groups and Allow_Group_Delay_Inhibit. A client's
Present_Value write or WriteGroup reaches the members over loopback UDP.
"""

from __future__ import annotations

import ast
import asyncio
import inspect
import time
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
    PropertyIdentifier as P,
    PropertyValue,
)

DEVICE_INSTANCE = 503_262
OWN_DEVICE = ObjectIdentifier(ObjectType.DEVICE, DEVICE_INSTANCE)
REMOTE_INSTANCE = 503_263
REMOTE_DEVICE = ObjectIdentifier(ObjectType.DEVICE, REMOTE_INSTANCE)
CH0 = ObjectIdentifier(ObjectType.CHANNEL, 0)
CH1 = ObjectIdentifier(ObjectType.CHANNEL, 1)
AO1 = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)
AO2 = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 2)
AV1 = ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)
PV = P.PRESENT_VALUE
LIST = P.LIST_OF_OBJECT_PROPERTY_REFERENCES

# Write_Status SUCCESSFUL.
SUCCESSFUL = 2


def member(oid: ObjectIdentifier, device: ObjectIdentifier | None = None) -> dict:
    """A member's Present_Value as a read gives it back (#1345): a
    DeviceObjectPropertyReference mapping with every key."""
    return {
        "object_identifier": oid,
        "property_identifier": PV,
        "property_array_index": None,
        "device_identifier": device,
    }
# An application-tagged REAL 72.0, a WriteGroup change-list value.
REAL_72 = bytes([0x44, 0x42, 0x90, 0x00, 0x00])
# Seconds the server gets to do what a test waits for. Each wait polls and
# returns as soon as the condition holds.
DEADLINE = 10.0

ARGUMENTS = [
    "instance",
    "name",
    "channel_number",
    "members",
    "execution_delay",
    "control_groups",
]


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


def make_server(instance: int = DEVICE_INSTANCE) -> BACnetServer:
    return BACnetServer(
        device_instance=instance,
        device_name=f"Channel Test {instance}",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )


async def distributed(read) -> None:
    """Poll CH-1's Write_Status until its distribution has succeeded."""
    deadline = time.monotonic() + DEADLINE
    while await read(CH1, P.WRITE_STATUS) != SUCCESSFUL:
        if time.monotonic() > deadline:
            raise AssertionError("the distribution never succeeded")
        await asyncio.sleep(0.01)


async def present_value(server: BACnetServer, oid: ObjectIdentifier):
    return (await server.read_property(oid, PV)).value


async def first_seen(server: BACnetServer, expected: dict) -> dict:
    """Poll each object's Present_Value until it holds the expected value;
    the `time.perf_counter()` time each was first seen holding it."""
    seen: dict = {}
    deadline = time.monotonic() + DEADLINE
    while len(seen) < len(expected):
        if time.monotonic() > deadline:
            missing = [str(oid) for oid in expected if oid not in seen]
            raise AssertionError(f"{missing} never took their values")
        for oid, value in expected.items():
            if oid not in seen and await present_value(server, oid) == value:
                seen[oid] = time.perf_counter()
        await asyncio.sleep(0.01)
    return seen


class ChannelStubContractTests(unittest.TestCase):
    def test_runtime_and_stub_take_the_same_arguments(self) -> None:
        parameters = inspect.signature(BACnetServer.add_channel).parameters
        self.assertEqual(
            list(parameters), ["self", *ARGUMENTS, "allow_group_delay_inhibit"]
        )
        for argument in ARGUMENTS:
            self.assertIs(
                parameters[argument].kind, inspect.Parameter.POSITIONAL_OR_KEYWORD
            )
        for argument in ARGUMENTS[3:]:
            self.assertIsNone(parameters[argument].default)
        inhibit = parameters["allow_group_delay_inhibit"]
        self.assertIs(inhibit.kind, inspect.Parameter.KEYWORD_ONLY)
        self.assertIs(inhibit.default, False)

        method = installed_stub_method("add_channel")
        self.assertEqual(
            [argument.arg for argument in method.args.args], ["self", *ARGUMENTS]
        )
        self.assertEqual(
            [argument.arg for argument in method.args.kwonlyargs],
            ["allow_group_delay_inhibit"],
        )
        for default in method.args.defaults:
            self.assertIsInstance(default, ast.Constant)
            self.assertIsNone(default.value)
        self.assertIs(method.args.kw_defaults[0].value, False)


class ChannelMembersTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.server = make_server()
        self.server.add_analog_output(1, "AO-1")
        self.server.add_analog_output(2, "AO-2")
        self.server.add_analog_value(1, "AV-1")

    async def run_with_client(self, check) -> None:
        await self.server.start()
        try:
            address = await self.server.local_address()
            async with BACnetClient(
                interface="127.0.0.1", port=0, apdu_timeout_ms=2000
            ) as client:

                async def read(oid, prop, index=None):
                    return (await client.read_property(address, oid, prop, index)).value

                await check(client, address, read)
        finally:
            await self.server.stop()

    async def test_present_value_write_reaches_both_members(self) -> None:
        self.server.add_channel(
            1, "CH-1", 11, [(AO1, PV), (AV1, PV, None)]
        )

        async def check(client, address, read) -> None:
            self.assertEqual(await read(CH1, P.CHANNEL_NUMBER), 11)
            self.assertEqual(await read(CH1, LIST, 0), 2)
            self.assertEqual(await read(CH1, P.EXECUTION_DELAY), [0, 0])
            await client.write_property(
                address, CH1, PV, PropertyValue.real(42.0), priority=8
            )
            await distributed(read)
            self.assertEqual(await read(AO1, PV), 42.0)
            self.assertEqual(await read(AV1, PV), 42.0)
            # Each member write carries the Channel write's priority.
            self.assertEqual(await read(AO1, P.PRIORITY_ARRAY, 8), 42.0)
            self.assertEqual(await read(CH1, P.LAST_PRIORITY), 8)

            # A local write starts a distribution too.
            await self.server.write_property_local(
                CH1, PV, PropertyValue.real(43.0), 8, source_object=None
            )
            await first_seen(self.server, {AO1: 43.0, AV1: 43.0})

        await self.run_with_client(check)

    async def test_member_in_another_device_is_written_there(self) -> None:
        remote = make_server(REMOTE_INSTANCE)
        remote.add_analog_output(1, "AO-1")
        await remote.start()
        try:
            self.server.add_device_binding(REMOTE_INSTANCE, await remote.local_address())
            remote_ao1 = {
                "object_identifier": AO1,
                "property_identifier": PV,
                "device_identifier": REMOTE_DEVICE,
            }
            self.server.add_channel(1, "CH-1", 11, [(AO1, PV), remote_ao1])

            async def check(client, address, read) -> None:
                # The member keeps its Device identifier.
                self.assertEqual(await read(CH1, LIST, 2), member(AO1, REMOTE_DEVICE))
                await client.write_property(
                    address, CH1, PV, PropertyValue.real(61.0), priority=8
                )
                await distributed(read)
                self.assertEqual(await present_value(remote, AO1), 61.0)
                self.assertEqual(await read(AO1, PV), 61.0)

            await self.run_with_client(check)
        finally:
            await remote.stop()

    async def test_execution_delay_holds_back_the_second_member(self) -> None:
        delay_ms = 400
        self.server.add_channel(
            1,
            "CH-1",
            11,
            members=[(AO1, PV), (AV1, PV)],
            execution_delay=[0, delay_ms],
        )

        async def check(client, address, read) -> None:
            self.assertEqual(await read(CH1, P.EXECUTION_DELAY), [0, delay_ms])
            # perf_counter, not monotonic: on Windows before Python 3.13,
            # monotonic moves in ~15.6 ms steps, so a difference of two
            # readings can come out short of the delay (#1454).
            start = time.perf_counter()
            await client.write_property(address, CH1, PV, PropertyValue.real(55.0))
            seen = await first_seen(self.server, {AO1: 55.0, AV1: 55.0})
            # The delay counts from the distribution's start, after `start`,
            # so AV-1 can't hold the value sooner. AO-1 has no delay.
            self.assertGreaterEqual(seen[AV1] - start, delay_ms / 1000)
            self.assertLessEqual(seen[AO1], seen[AV1])

        await self.run_with_client(check)

    async def test_write_group_reaches_the_channel_through_its_control_groups(
        self,
    ) -> None:
        # CH-0 has the same number but is in no group. The server plans
        # Channels in instance order, so a value it wrongly took would land
        # before CH-1's.
        self.server.add_channel(0, "CH-0", 11, [(AO2, PV)])
        own_device_av1 = {
            "object_identifier": AV1,
            "property_identifier": PV,
            "device_identifier": OWN_DEVICE,
        }
        self.server.add_channel(
            1,
            "CH-1",
            11,
            [(AO1, PV), own_device_av1],
            # A delay the WriteGroup's Inhibit Delay skips: AV-1 would
            # otherwise wait out the whole deadline.
            execution_delay=[0, 60_000],
            control_groups=[5, 7],
            allow_group_delay_inhibit=True,
        )

        async def check(client, address, read) -> None:
            self.assertEqual(await read(CH1, P.CONTROL_GROUPS), [5, 7])
            self.assertIs(await read(CH1, P.ALLOW_GROUP_DELAY_INHIBIT), True)
            self.assertEqual(await read(CH0, P.CONTROL_GROUPS), [0])
            # The member naming this Device is stored as the local reference.
            self.assertEqual(await read(CH1, LIST, 2), member(AV1))

            await client.write_group(address, 7, 8, [(11, None, REAL_72)], True)
            await first_seen(self.server, {AO1: 72.0, AV1: 72.0})
            self.assertIsNone(await read(CH0, PV))
            self.assertEqual(await read(AO2, PV), 0.0)

        await self.run_with_client(check)


class ChannelWildcardDeviceTests(unittest.IsolatedAsyncioTestCase):
    async def test_a_member_naming_the_wildcard_device_keeps_it(self) -> None:
        # Instance 4194303 names no particular device, so a member naming
        # Device 4194303 isn't this server's and keeps its device.
        wildcard = ObjectIdentifier(ObjectType.DEVICE, 4_194_303)
        server = make_server()
        server.add_analog_value(1, "AV-1")
        server.add_channel(
            1,
            "CH-1",
            11,
            [
                {
                    "object_identifier": AV1,
                    "property_identifier": PV,
                    "device_identifier": wildcard,
                }
            ],
        )
        await server.start()
        try:
            element = (await server.read_property(CH1, LIST, 1)).value
            self.assertEqual(element, member(AV1, wildcard))
        finally:
            await server.stop()

    async def test_a_server_cannot_be_the_wildcard_device(self) -> None:
        # No object may have instance 4194303 (#2171), the Device included,
        # so the constructor refuses it.
        with self.assertRaisesRegex(ValueError, "4194303"):
            make_server(4_194_303)


class ChannelArgumentTests(unittest.IsolatedAsyncioTestCase):
    async def test_bad_arguments_raise_and_register_nothing(self) -> None:
        server = make_server()
        instances = iter(range(10, 1000))
        refused: list[int] = []

        def refuse(error, *args, **kwargs):
            instance = next(instances)
            refused.append(instance)
            with self.assertRaises(error) as raised:
                server.add_channel(instance, f"CH-{instance}", *args, **kwargs)
            return raised.exception

        mapping = {"object_identifier": AV1, "property_identifier": PV}
        type_errors = (
            ("a str for the list", {"members": "AO-1"}),
            ("a bare object", {"members": [AO1]}),
            ("one item", {"members": [(AO1,)]}),
            ("four items", {"members": [(AO1, PV, 1, 2)]}),
            ("items out of order", {"members": [(PV, AO1)]}),
            ("a numeric property", {"members": [(AO1, 85)]}),
            ("a str index", {"members": [(AO1, PV, "1")]}),
            ("a list member", {"members": [[AO1, PV]]}),
            ("a mapping's numeric property", {"members": [dict(mapping, property_identifier=85)]}),
            ("delays not a list", {"members": [(AO1, PV)], "execution_delay": "fast"}),
            ("an int flag", {"allow_group_delay_inhibit": 1}),
        )
        for case, kwargs in type_errors:
            with self.subTest(case=case):
                refuse(TypeError, 11, **kwargs)
        with self.subTest(case="a str channel number"):
            refuse(TypeError, "11")
        with self.subTest(case="the flag given by position"):
            refuse(TypeError, 11, None, None, None, True)

        value_errors = (
            ("an unknown key", dict(mapping, priority=8)),
            ("a missing key", {"object_identifier": AV1}),
            ("a device that isn't a Device", dict(mapping, device_identifier=AO1)),
        )
        for case, member in value_errors:
            with self.subTest(case=case):
                refuse(ValueError, 11, [member])

        # Outside its integer type, a tuple's or a mapping's alike (#1360).
        overflow_errors = (
            ("a negative channel number", (-1,), {}),
            ("a channel number past 65535", (65_536,), {}),
            ("a mapping's negative index", (11,),
             {"members": [dict(mapping, property_array_index=-1)]}),
            ("a negative index", (11,), {"members": [(AO1, PV, -1)]}),
            ("an index past unsigned32", (11,), {"members": [(AO1, PV, 1 << 32)]}),
            ("a negative delay", (11,), {"members": [(AO1, PV)], "execution_delay": [-1]}),
            ("a group past unsigned32", (11,), {"control_groups": [1 << 32]}),
        )
        for case, args, kwargs in overflow_errors:
            with self.subTest(case=case):
                refuse(OverflowError, *args, **kwargs)

        protocol_errors = (
            (
                "two delays for one member",
                (11,),
                {"members": [(AO1, PV)], "execution_delay": [0, 100]},
                ErrorCode.VALUE_OUT_OF_RANGE,
            ),
            ("no group", (11,), {"control_groups": []}, ErrorCode.VALUE_OUT_OF_RANGE),
            (
                "65 groups",
                (11,),
                {"control_groups": list(range(1, 66))},
                ErrorCode.NO_SPACE_TO_WRITE_PROPERTY,
            ),
            (
                "1025 members",
                (11,),
                {"members": [(AO1, PV)] * 1025},
                ErrorCode.NO_SPACE_TO_WRITE_PROPERTY,
            ),
        )
        for case, args, kwargs, code in protocol_errors:
            with self.subTest(case=case):
                error = refuse(BacnetProtocolError, *args, **kwargs)
                self.assertEqual(error.error_code, code.to_raw())

        await server.start()
        try:
            for instance in refused:
                with self.assertRaises(BacnetProtocolError) as raised:
                    await server.read_property(
                        ObjectIdentifier(ObjectType.CHANNEL, instance), P.OBJECT_NAME
                    )
                self.assertEqual(
                    raised.exception.error_code, ErrorCode.UNKNOWN_OBJECT.to_raw()
                )
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
