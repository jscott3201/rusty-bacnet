"""Installed-artifact tests for the Program_State application route (#2258).

Program_State is read-only on the network and to `write_property_local`;
`set_program_state_local` is how the application reports it.
"""

from __future__ import annotations

import asyncio
import inspect
import unittest

from rusty_bacnet import (
    BACnetServer,
    BacnetProtocolError,
    ErrorClass,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

PROGRAM = ObjectIdentifier(ObjectType.PROGRAM, 1)
AV = ObjectIdentifier(ObjectType.ANALOG_VALUE, 2)
PS = PropertyIdentifier.PROGRAM_STATE


class ProgramStateLocalTests(unittest.TestCase):
    def test_signature(self) -> None:
        self.assertEqual(
            list(inspect.signature(BACnetServer.set_program_state_local).parameters),
            ["self", "object_id", "value"],
        )

    def test_live_server_reports_program_state(self) -> None:
        asyncio.run(self._exercise())

    def assert_code(self, raised: BacnetProtocolError, cls: ErrorClass, code: ErrorCode) -> None:
        self.assertEqual(raised.error_class, cls.to_raw())
        self.assertEqual(raised.error_code, code.to_raw())

    async def _exercise(self) -> None:
        server = BACnetServer(
            device_instance=2_258_001,
            device_name="Program State Test",
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        )
        server.add_program(1, "PRG-1")
        server.add_analog_value(2, "AV-2")
        with self.assertRaises(RuntimeError):
            await server.set_program_state_local(PROGRAM, PropertyValue.enumerated(2))
        await server.start()
        try:
            with self.assertRaises(BacnetProtocolError) as raised:
                await server.write_property_local(
                    PROGRAM, PS, PropertyValue.enumerated(2), source_object=None
                )
            self.assert_code(
                raised.exception, ErrorClass.PROPERTY, ErrorCode.WRITE_ACCESS_DENIED
            )
            self.assertIsNone(
                await server.set_program_state_local(PROGRAM, PropertyValue.enumerated(2))
            )
            self.assertEqual(
                await server.read_property(PROGRAM, PS), PropertyValue.enumerated(2)
            )
            for value, code in (
                (PropertyValue.enumerated(6), ErrorCode.VALUE_OUT_OF_RANGE),
                (PropertyValue.unsigned(1), ErrorCode.INVALID_DATA_TYPE),
            ):
                with self.subTest(code=code):
                    with self.assertRaises(BacnetProtocolError) as raised:
                        await server.set_program_state_local(PROGRAM, value)
                    self.assert_code(raised.exception, ErrorClass.PROPERTY, code)
            with self.assertRaises(BacnetProtocolError) as raised:
                await server.set_program_state_local(AV, PropertyValue.enumerated(1))
            self.assert_code(
                raised.exception,
                ErrorClass.OBJECT,
                ErrorCode.OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            )
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
