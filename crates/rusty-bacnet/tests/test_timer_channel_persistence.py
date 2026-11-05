"""Installed-artifact tests for add_timer's and add_channel's storage_path (#2312).

What a client writes over the network to a Timer's or a Channel's
configuration, or Description (#2364), is saved in the file, and a server rebuilt on the same files
serves it over the keyword arguments, until clear_written_config drops it.
"""

from __future__ import annotations

import inspect
import tempfile
import unittest
from pathlib import Path

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier as P,
    PropertyValue,
)

DEVICE_INSTANCE = 523_312
TMR1 = ObjectIdentifier(ObjectType.TIMER, 1)
CH1 = ObjectIdentifier(ObjectType.CHANNEL, 1)
AO1 = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)
PV = P.PRESENT_VALUE
LIST = P.LIST_OF_OBJECT_PROPERTY_REFERENCES


def reference(instance: int) -> bytes:
    """Analog Output `instance`'s Present_Value as an encoded
    BACnetDeviceObjectPropertyReference: the object identifier in context
    tag 0, the property (85) in context tag 1."""
    object_identifier = (ObjectType.ANALOG_OUTPUT.to_raw() << 22) | instance
    return bytes([0x0C]) + object_identifier.to_bytes(4, "big") + bytes([0x19, 85])


# Each covered property: (object, property, array index, value written).
WRITES = [
    (TMR1, P.STATE_CHANGE_VALUES, 1, PropertyValue.real(42.0)),
    (TMR1, LIST, None, PropertyValue.application_data(reference(2))),
    (TMR1, P.DEFAULT_TIMEOUT, None, PropertyValue.unsigned(5_000)),
    (CH1, LIST, None, PropertyValue.application_data(reference(2))),
    (CH1, P.EXECUTION_DELAY, 1, PropertyValue.unsigned(250)),
    (CH1, P.CHANNEL_NUMBER, None, PropertyValue.unsigned(77)),
    (CH1, P.CONTROL_GROUPS, 1, PropertyValue.unsigned(9)),
    (CH1, P.ALLOW_GROUP_DELAY_INHIBIT, None, PropertyValue.boolean(True)),
    # Kept beside the configuration (#2364).
    (TMR1, P.DESCRIPTION, None, PropertyValue.character_string("Lobby")),
    (CH1, P.DESCRIPTION, None, PropertyValue.character_string("Lobby")),
]
# What a read compares: each covered property, and Priority_For_Writing,
# kept with the Timer's two.
SERVED = [(oid, prop) for oid, prop, _, _ in WRITES] + [(TMR1, P.PRIORITY_FOR_WRITING)]


class TimerChannelStorageTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.directory = Path(directory.name)

    def build(self, clear: bool = False) -> BACnetServer:
        server = BACnetServer(
            device_instance=DEVICE_INSTANCE,
            device_name="Storage Test",
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        )
        server.add_analog_output(1, "AO-1")
        server.add_analog_output(2, "AO-2")
        server.add_timer(
            1,
            "TMR-1",
            state_change_values=[PropertyValue.real(1.0)] + [None] * 6,
            references=[(AO1, PV)],
            priority_for_writing=9,
            storage_path=str(self.directory / "timer-1"),
            clear_written_config=clear,
        )
        server.add_channel(
            1,
            "CH-1",
            11,
            [(AO1, PV)],
            [0],
            [5],
            storage_path=str(self.directory / "channel-1"),
            clear_written_config=clear,
        )
        return server

    async def served(self, clear: bool = False) -> list:
        """What a server built on the files serves of SERVED."""
        server = self.build(clear)
        await server.start()
        try:
            return [(await server.read_property(oid, prop)).value for oid, prop in SERVED]
        finally:
            await server.stop()

    async def write_over_network(self) -> tuple[list, list]:
        """Make WRITES through a client: what was served before them, and
        after."""
        server = self.build()
        await server.start()
        try:
            configured = [
                (await server.read_property(oid, prop)).value for oid, prop in SERVED
            ]
            address = await server.local_address()
            async with BACnetClient(
                interface="127.0.0.1", port=0, apdu_timeout_ms=2000
            ) as client:
                for oid, prop, index, value in WRITES:
                    await client.write_property(address, oid, prop, value, array_index=index)
            written = [(await server.read_property(oid, prop)).value for oid, prop in SERVED]
        finally:
            await server.stop()
        for (oid, prop), before, after in zip(SERVED[:-1], configured, written):
            with self.subTest(object=oid, property=prop):
                self.assertNotEqual(before, after)
        return configured, written

    async def test_network_writes_survive_a_rebuild(self) -> None:
        _, written = await self.write_over_network()
        for boot in ("first", "second"):
            with self.subTest(boot=boot):
                self.assertEqual(await self.served(), written)

    async def test_clear_written_config_lets_the_keywords_apply(self) -> None:
        configured, _ = await self.write_over_network()
        # At the start that clears, and the next.
        self.assertEqual(await self.served(clear=True), configured)
        self.assertEqual(await self.served(), configured)

    def test_the_storage_keywords_are_keyword_only(self) -> None:
        for method in (BACnetServer.add_timer, BACnetServer.add_channel):
            parameters = inspect.signature(method).parameters
            with self.subTest(method=method.__name__):
                self.assertEqual(
                    list(parameters)[-2:], ["storage_path", "clear_written_config"]
                )
                self.assertIs(
                    parameters["storage_path"].kind, inspect.Parameter.KEYWORD_ONLY
                )
                self.assertIsNone(parameters["storage_path"].default)
                clear = parameters["clear_written_config"]
                self.assertIs(clear.kind, inspect.Parameter.KEYWORD_ONLY)
                self.assertIs(clear.default, False)


if __name__ == "__main__":
    unittest.main()
