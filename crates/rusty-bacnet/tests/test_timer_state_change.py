"""Installed-artifact tests for add_timer's State_Change_Values keywords (#2206).

A Timer given state_change_values and references writes each transition's
value to its references at priority_for_writing.
"""

from __future__ import annotations

import asyncio
import inspect
import time
import unittest

from rusty_bacnet import (
    BACnetServer,
    BacnetProtocolError,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier as P,
    PropertyValue,
)

TMR1 = ObjectIdentifier(ObjectType.TIMER, 1)
AO1 = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)
PV = P.PRESENT_VALUE
DEADLINE = 10.0
# IDLE_TO_RUNNING writes 10.0, RUNNING_TO_IDLE relinquishes, the rest nothing.
VALUES = [PropertyValue.real(10.0), PropertyValue.null(), None, None, None, None, None]


def make_server() -> BACnetServer:
    return BACnetServer(
        device_instance=522_206,
        device_name="Timer Test",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )


async def slot(server: BACnetServer, index: int):
    return (await server.read_property(AO1, P.PRIORITY_ARRAY, index)).value


async def until(server: BACnetServer, expected) -> None:
    deadline = time.monotonic() + DEADLINE
    while await slot(server, 9) != expected:
        if time.monotonic() > deadline:
            raise AssertionError(f"AO-1 slot 9 never held {expected!r}")
        await asyncio.sleep(0.01)


class TimerStateChangeTests(unittest.IsolatedAsyncioTestCase):
    def test_the_keywords_are_keyword_only(self) -> None:
        parameters = inspect.signature(BACnetServer.add_timer).parameters
        self.assertEqual(
            list(parameters),
            [
                "self",
                "instance",
                "name",
                "state_change_values",
                "references",
                "priority_for_writing",
                "storage_path",
                "clear_written_config",
            ],
        )
        for name in ["state_change_values", "references", "priority_for_writing"]:
            self.assertIs(parameters[name].kind, inspect.Parameter.KEYWORD_ONLY)
        self.assertIsNone(parameters["priority_for_writing"].default)

    async def test_transitions_write_their_values_at_the_priority(self) -> None:
        server = make_server()
        server.add_analog_output(1, "AO-1")
        server.add_timer(
            1,
            "TMR-1",
            state_change_values=VALUES,
            references=[(AO1, PV)],
            priority_for_writing=9,
        )
        # A plain Timer has none of the three.
        server.add_timer(2, "TMR-2")
        await server.start()
        try:
            self.assertEqual(
                (await server.read_property(TMR1, P.PRIORITY_FOR_WRITING)).value, 9
            )
            with self.assertRaises(BacnetProtocolError) as raised:
                await server.read_property(
                    ObjectIdentifier(ObjectType.TIMER, 2), P.PRIORITY_FOR_WRITING
                )
            self.assertEqual(
                raised.exception.error_code, ErrorCode.UNKNOWN_PROPERTY.to_raw()
            )
            await server.write_property_local(
                TMR1, PV, PropertyValue.unsigned(60_000), source_object=None
            )
            await until(server, 10.0)
            await server.write_property_local(
                TMR1, P.TIMER_STATE, PropertyValue.enumerated(0), source_object=None
            )
            await until(server, None)
        finally:
            await server.stop()

    def test_refusals(self) -> None:
        server = make_server()
        with self.assertRaises(ValueError):
            server.add_timer(1, "TMR-1", state_change_values=VALUES[:6])
        with self.assertRaises(TypeError):
            server.add_timer(1, "TMR-1", state_change_values=[1] * 7)
        with self.assertRaises(ValueError):
            server.add_timer(1, "TMR-1", priority_for_writing=8)
        for priority in [0, 17, 256, 1 << 40]:
            with self.subTest(priority=priority):
                with self.assertRaises(BacnetProtocolError) as raised:
                    server.add_timer(
                        1, "TMR-1", references=[(AO1, PV)], priority_for_writing=priority
                    )
                self.assertEqual(
                    raised.exception.error_code, ErrorCode.VALUE_OUT_OF_RANGE.to_raw()
                )


if __name__ == "__main__":
    unittest.main()
