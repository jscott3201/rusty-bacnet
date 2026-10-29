"""EngineeringUnits, ErrorClass, ErrorCode, RejectReason and AbortReason are
IntEnum classes (#1816).

Members are ints: they compare equal to their numbers, hash like them and go
wherever an int goes, so code written against plain ints keeps working. A
value without a name is a pseudo-member of its class. Object constructors take
a member or an int for ``units``, and a raised error's ``error_class`` and
``error_code`` are members.
"""

from __future__ import annotations

import asyncio
import copy
import enum
import pickle
import unittest

from rusty_bacnet import (
    AbortReason, AuditOperation, BACnetClient, BACnetServer, BacnetProtocolError, EngineeringUnits, ErrorClass,
    ErrorCode, ObjectIdentifier, ObjectType, PropertyIdentifier, PropertyValue,
    RejectReason,
)

CLASSES = {
    EngineeringUnits: 2**32 - 1,
    ErrorClass: 2**16 - 1,
    ErrorCode: 2**16 - 1,
    RejectReason: 2**8 - 1,
    AbortReason: 2**8 - 1,
}


class IntEnumClassTests(unittest.TestCase):
    def test_members_are_ints_that_equal_their_numbers(self) -> None:
        for cls in CLASSES:
            with self.subTest(enum=cls.__name__):
                self.assertTrue(issubclass(cls, enum.IntEnum))
                self.assertEqual(cls.__module__, "rusty_bacnet")
                for member in cls:
                    self.assertIsInstance(member, int)
                    self.assertEqual(member, int(member))
                    self.assertEqual(member.to_raw(), int(member))
                    self.assertEqual(hash(member), hash(int(member)))
                    self.assertIs(cls(int(member)), member)
                    self.assertIs(cls.from_raw(int(member)), member)
                    self.assertEqual(repr(member), f"{cls.__name__}.{member.name}")
                    self.assertEqual(str(member), str(int(member)))

    def test_known_values(self) -> None:
        self.assertEqual(EngineeringUnits.DEGREES_CELSIUS, 62)
        self.assertEqual(ErrorClass.PROPERTY, 2)
        self.assertEqual(ErrorCode.UNKNOWN_OBJECT, 31)
        self.assertEqual(RejectReason.UNRECOGNIZED_SERVICE, 9)
        self.assertEqual(AbortReason.SEGMENTATION_NOT_SUPPORTED, 4)
        self.assertEqual({31: "found"}[ErrorCode.UNKNOWN_OBJECT], "found")
        self.assertEqual((ErrorClass.OBJECT, ErrorCode.UNKNOWN_OBJECT), (1, 31))

    def test_a_value_without_a_name_is_a_pseudo_member(self) -> None:
        for cls, top in CLASSES.items():
            with self.subTest(enum=cls.__name__):
                named = {int(member) for member in cls}
                raw = next(n for n in (200, 1000, top) if n not in named)
                value = cls.from_raw(raw)
                self.assertIs(type(value), cls)
                self.assertEqual(value, raw)
                self.assertEqual(cls(raw), value)
                self.assertIsNone(value.name)
                self.assertEqual(value.to_raw(), raw)
                self.assertEqual(repr(value), f"{cls.__name__}({raw})")
                for copied in [
                    copy.copy(value),
                    copy.deepcopy(value),
                    *(pickle.loads(pickle.dumps(value, p))
                      for p in range(pickle.HIGHEST_PROTOCOL + 1)),
                ]:
                    self.assertIs(type(copied), cls)
                    self.assertEqual(copied, value)

    def test_a_number_outside_the_range_is_refused(self) -> None:
        for cls, top in CLASSES.items():
            for raw in (-1, top + 1):
                with self.subTest(enum=cls.__name__, raw=raw), self.assertRaises(ValueError):
                    cls.from_raw(raw)


class ErrorPairInputTests(unittest.TestCase):
    """An (ErrorClass, ErrorCode) input takes members or ints in range: an
    int out of range is a ValueError, a value of the wrong type a
    TypeError. Both are refused before anything is sent."""

    def test_audit_result_range_and_type(self) -> None:
        client = BACnetClient()
        device = lambda instance: {  # noqa: E731
            "kind": "device",
            "object_identifier": ObjectIdentifier(ObjectType.DEVICE, instance),
        }

        def send(result):
            notification = {
                "source_device": device(1),
                "operation": AuditOperation.READ,
                "target_device": device(2),
                "result": result,
            }
            return client.confirmed_audit_notification_typed(
                "127.0.0.1:47808", {"notifications": [notification]}
            )

        for result in [(2, 2**16), (ErrorClass.PROPERTY, -1)]:
            with self.subTest(result=result), self.assertRaises(ValueError):
                send(result)
        for result in [(True, 0), (ErrorClass.PROPERTY, "32")]:
            with self.subTest(result=result), self.assertRaises(TypeError):
                send(result)


class IntEnumApiTests(unittest.IsolatedAsyncioTestCase):
    async def test_units_take_a_member_or_an_int(self) -> None:
        server = BACnetServer(9873, interface="127.0.0.1", port=0)
        server.add_analog_input(1, "Member", units=EngineeringUnits.DEGREES_CELSIUS)
        server.add_analog_input(2, "Int", units=62)
        server.add_analog_value(3, "Unnamed", units=EngineeringUnits.from_raw(1000))
        await server.start()
        try:
            units = PropertyIdentifier.UNITS
            for oid, expected in [
                (ObjectIdentifier(ObjectType.ANALOG_INPUT, 1), 62),
                (ObjectIdentifier(ObjectType.ANALOG_INPUT, 2), 62),
                (ObjectIdentifier(ObjectType.ANALOG_VALUE, 3), 1000),
            ]:
                self.assertEqual(
                    await server.read_property(oid, units),
                    PropertyValue.enumerated(expected),
                )
        finally:
            await server.stop()

    async def test_a_raised_error_carries_members(self) -> None:
        server = BACnetServer(9874, interface="127.0.0.1", port=0)
        await server.start()
        try:
            missing = ObjectIdentifier(ObjectType.ANALOG_INPUT, 99)
            with self.assertRaises(BacnetProtocolError) as raised:
                await asyncio.wait_for(
                    server.read_property(missing, PropertyIdentifier.PRESENT_VALUE), 3
                )
        finally:
            await server.stop()
        error = raised.exception
        self.assertIs(error.error_class, ErrorClass.OBJECT)
        self.assertIs(error.error_code, ErrorCode.UNKNOWN_OBJECT)
        # Plain-int comparisons keep working.
        self.assertEqual((error.error_class, error.error_code), (1, 31))
        self.assertIsInstance(error.error_code, int)


if __name__ == "__main__":
    unittest.main()
