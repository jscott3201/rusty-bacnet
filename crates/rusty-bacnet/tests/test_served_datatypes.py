"""Installed-artifact tests for two served datatypes (#2251, #2252).

An Accumulator's Pulse_Rate is an Unsigned (Table 12-79), and a Life Safety
Point's Maintenance_Required a BACnetMaintenance enumeration (Table 12-18),
read with its name. Writes of the datatypes they were once served as, a REAL
and a BOOLEAN, are refused with INVALID_DATA_TYPE.
"""

from __future__ import annotations

import asyncio
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
POINT = ObjectIdentifier(ObjectType.LIFE_SAFETY_POINT, 1)
PULSE_RATE = PropertyIdentifier.PULSE_RATE
MAINTENANCE = PropertyIdentifier.MAINTENANCE_REQUIRED


def make_server() -> BACnetServer:
    server = BACnetServer(
        device_instance=2_251_001,
        device_name="Served Datatypes Artifact Test",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )
    server.add_accumulator(1, "ACC-1", 95)
    server.add_life_safety_point(1, "LSP-1")
    return server


class ServedDatatypeTests(unittest.TestCase):
    def test_pulse_rate_and_maintenance_on_the_wire(self) -> None:
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
                rate = await client.read_property(address, ACCUMULATOR, PULSE_RATE)
                self.assertEqual(rate.tag, "unsigned")
                self.assertEqual(rate.value, 0)
                await client.write_property(
                    address, ACCUMULATOR, PULSE_RATE, PropertyValue.unsigned(25)
                )
                self.assertEqual(
                    await client.read_property(address, ACCUMULATOR, PULSE_RATE),
                    PropertyValue.unsigned(25),
                )

                maintenance = await client.read_property(address, POINT, MAINTENANCE)
                self.assertEqual(maintenance.tag, "enumerated")
                self.assertEqual(maintenance.value, 0)
                self.assertEqual(maintenance.name, "NONE")
                await client.write_property(
                    address, POINT, MAINTENANCE, PropertyValue.enumerated(2)
                )
                maintenance = await client.read_property(address, POINT, MAINTENANCE)
                self.assertEqual(maintenance.value, 2)
                self.assertEqual(maintenance.name, "NEED_SERVICE_OPERATIONAL")

                for target, prop, value in (
                    (ACCUMULATOR, PULSE_RATE, PropertyValue.real(2.5)),
                    (POINT, MAINTENANCE, PropertyValue.boolean(True)),
                ):
                    with self.subTest(refused=value):
                        with self.assertRaises(BacnetProtocolError) as raised:
                            await client.write_property(address, target, prop, value)
                        self.assertEqual(
                            raised.exception.error_class, ErrorClass.PROPERTY.to_raw()
                        )
                        self.assertEqual(
                            raised.exception.error_code,
                            ErrorCode.INVALID_DATA_TYPE.to_raw(),
                        )
                self.assertEqual(
                    await client.read_property(address, ACCUMULATOR, PULSE_RATE),
                    PropertyValue.unsigned(25),
                )
                self.assertEqual(
                    (await client.read_property(address, POINT, MAINTENANCE)).value, 2
                )
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
