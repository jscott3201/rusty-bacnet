"""Installed-artifact tests for feeding pulses to an Accumulator (#2259).

`add_pulses_local` steps Present_Value through Prescale, wrapping past
Max_Pres_Value, and the running server turns the pulses of each
Limit_Monitoring_Interval into Pulse_Rate. Out of service, a client's values
stay.
"""

from __future__ import annotations

import asyncio
import inspect
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


ACCUMULATOR = ObjectIdentifier(ObjectType.ACCUMULATOR, 1)
AV = ObjectIdentifier(ObjectType.ANALOG_VALUE, 2)
PV = PropertyIdentifier.PRESENT_VALUE
PULSE_RATE = PropertyIdentifier.PULSE_RATE
RATE_TIMEOUT = 5.0


def make_server() -> BACnetServer:
    server = BACnetServer(
        device_instance=2_259_001,
        device_name="Accumulator Pulse Feed Artifact Test",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )
    # Two counts for every three pulses.
    server.add_accumulator(1, "ACC-1", 95, prescale=(2, 3))
    server.add_analog_value(2, "AV-2")
    return server


class AccumulatorPulseFeedTests(unittest.TestCase):
    def test_signature(self) -> None:
        self.assertEqual(
            list(inspect.signature(BACnetServer.add_pulses_local).parameters),
            ["self", "object_id", "pulses"],
        )

    def test_pulses_step_present_value_and_pulse_rate(self) -> None:
        asyncio.run(self._exercise())

    async def _exercise(self) -> None:
        server = make_server()
        await server.start()
        try:
            address = await server.local_address()
            async with BACnetClient(
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
                apdu_timeout_ms=2_000,
            ) as client:

                async def read(property_id: PropertyIdentifier) -> int:
                    value = await client.read_property(address, ACCUMULATOR, property_id)
                    self.assertEqual(value.tag, "unsigned")
                    return value.value

                async def write(property_id: PropertyIdentifier, value: PropertyValue) -> None:
                    await server.write_property_local(
                        ACCUMULATOR, property_id, value, source_object=None
                    )

                await write(PropertyIdentifier.MAX_PRES_VALUE, PropertyValue.unsigned(9))
                # 13 pulses are 8 counts with one pulse held toward the next.
                await server.add_pulses_local(ACCUMULATOR, 13)
                self.assertEqual(await read(PV), 8)
                # Two more make 10 counts, which wrap past 9 to 0.
                await server.add_pulses_local(ACCUMULATOR, 2)
                self.assertEqual(await read(PV), 0)

                # A one-second window: the rate reads the pulses fed into it
                # once it closes.
                await write(
                    PropertyIdentifier.LIMIT_MONITORING_INTERVAL,
                    PropertyValue.unsigned(1),
                )
                await server.add_pulses_local(ACCUMULATOR, 42)
                deadline = time.monotonic() + RATE_TIMEOUT
                while await read(PULSE_RATE) != 42:
                    self.assertLess(time.monotonic(), deadline, "Pulse_Rate never read 42")
                    await asyncio.sleep(0.05)

                # Out of service, the values a client wrote stay.
                await write(PropertyIdentifier.OUT_OF_SERVICE, PropertyValue.boolean(True))
                await write(PULSE_RATE, PropertyValue.unsigned(500))
                held = await read(PV)
                await server.add_pulses_local(ACCUMULATOR, 30)
                await asyncio.sleep(1.2)
                self.assertEqual(await read(PULSE_RATE), 500)
                self.assertEqual(await read(PV), held)

                with self.assertRaises(BacnetProtocolError) as raised:
                    await server.add_pulses_local(AV, 1)
                self.assertEqual(raised.exception.error_class, ErrorClass.OBJECT.to_raw())
                self.assertEqual(
                    raised.exception.error_code,
                    ErrorCode.OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw(),
                )
                with self.assertRaises(OverflowError):
                    await server.add_pulses_local(ACCUMULATOR, -1)
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
