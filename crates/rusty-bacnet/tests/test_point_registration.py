"""One registration path for the point builders of BACnetServer and the
endpoints (#2711): the same checks, with the same exceptions, at ``add_*``."""

import math
import tempfile
import unittest
from pathlib import Path

import rusty_bacnet as rb

P = rb.PropertyIdentifier
O = rb.ObjectType

KINDS = (
    "analog_input", "analog_output", "analog_value",
    "binary_input", "binary_output", "binary_value",
    "multistate_input", "multistate_output", "multistate_value",
)


def register(owner, kind, instance, name, *args):
    """Register one point; a multi-state point gets three states."""
    if kind.startswith("multistate") and not args:
        args = (3,)
    return getattr(owner, "add_" + kind)(instance, name, *args)


def server():
    return rb.BACnetServer(2711, device_name="Host", interface="127.0.0.1", port=0)


def owners():
    return {
        "server": server(),
        "bip": rb.BipEndpoint(2711, device_name="Host", interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1"),
        "mstp": rb.MstpEndpoint(2711, device_name="Host", serial_port="unused"),
    }


def outcome(call):
    """The exception class and text a call raises, or None."""
    try:
        call()
    except Exception as error:  # noqa: BLE001 - every refusal is compared
        return type(error), str(error)
    return None


class SameChecksTests(unittest.TestCase):
    CASES = [
        *[(f"analog input {value}", lambda o, v=value: o.add_analog_input(5, "Bad", 62, v))
          for value in (math.nan, math.inf, -math.inf)],
        *[(f"{kind} instance {instance}", lambda o, k=kind, i=instance: register(o, k, i, "Far"))
          for kind in KINDS for instance in (4194303, 4194304)],
        *[(f"{kind} named as the Device", lambda o, k=kind: register(o, k, 6, "Host"))
          for kind in KINDS],
        *[(f"{kind} with a taken name", lambda o, k=kind: register(o, k, 7, "Kept"))
          for kind in KINDS],
        ("a taken identifier", lambda o: o.add_analog_input(1, "Other")),
        *[(f"{kind} with {states} states", lambda o, k=kind, n=states: register(o, k, 8, "N", n))
          for kind in KINDS if kind.startswith("multistate")
          for states in (0, 1025, 4_000_000_000)],
    ]

    def test_every_surface_refuses_the_same_registrations_alike(self):
        results = {}
        for label, owner in owners().items():
            owner.add_analog_input(1, "Kept", 62, 1.5)
            for case, call in self.CASES:
                results.setdefault(case, {})[label] = outcome(lambda: call(owner))
            # A refusal keeps nothing and drops nothing.
            self.assertEqual(owner._pending_registration_count(), 1, label)
        for case, by_owner in results.items():
            with self.subTest(case=case):
                self.assertIsNotNone(by_owner["server"], by_owner)
                self.assertEqual(len(set(by_owner.values())), 1, by_owner)

    def test_the_refusals_name_what_was_wrong(self):
        for label, owner in owners().items():
            with self.subTest(owner=label):
                owner.add_analog_input(1, "Kept", 62, 1.5)
                for value in (math.nan, math.inf):
                    with self.assertRaises(rb.BacnetProtocolError) as raised:
                        owner.add_analog_input(2, "Bad", 62, value)
                    self.assertEqual(raised.exception.error_code,
                                     rb.ErrorCode.VALUE_OUT_OF_RANGE.to_raw())
                with self.assertRaisesRegex(rb.BacnetError, "4194303"):
                    owner.add_binary_value(4194303, "Reserved")
                with self.assertRaisesRegex(ValueError, "^duplicate object name: Host$"):
                    owner.add_multistate_output(2, "Host", 2)
                with self.assertRaisesRegex(ValueError, "^duplicate object name: Kept$"):
                    owner.add_binary_input(2, "Kept")
                with self.assertRaisesRegex(ValueError, "^duplicate object identifier"):
                    owner.add_analog_input(1, "Another")
                with self.assertRaisesRegex(rb.BacnetError, "at most 1024"):
                    owner.add_multistate_input(2, "Too many", 1025)
                # The cap itself is taken.
                owner.add_multistate_value(2, "Most", 1024)
                self.assertEqual(owner._pending_registration_count(), 2)

    def test_a_refused_server_registration_leaves_its_storage_file_alone(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "reporting"
            # Not a storage file: a build would refuse it, naming the path.
            path.write_bytes(b"not a storage file")
            owner = server()
            owner.add_analog_input(1, "Kept", 62, 1.0)
            with self.assertRaisesRegex(ValueError, "duplicate object name"):
                owner.add_analog_value(2, "Kept", storage_path=str(path),
                                       clear_written_reporting=True)
            with self.assertRaisesRegex(rb.BacnetError, "4194303"):
                owner.add_multistate_value(4194303, "Reserved", 2, storage_path=str(path),
                                           clear_written_reporting=True)
            self.assertEqual(path.read_bytes(), b"not a storage file")
            # The checks passed, the build reads the file and refuses it.
            with self.assertRaisesRegex(rb.BacnetError, "no valid header"):
                owner.add_analog_value(2, "Fresh", storage_path=str(path))
            self.assertEqual(owner._pending_registration_count(), 1)


class ServerStartTests(unittest.IsolatedAsyncioTestCase):
    async def test_registrations_a_refusal_left_are_served(self):
        owner = server()
        owner.add_analog_input(1, "Kept", 62, 1.5)
        with self.assertRaises(ValueError):
            owner.add_binary_value(1, "Kept")
        with self.assertRaises(rb.BacnetProtocolError):
            owner.add_analog_input(2, "NaN", 62, math.nan)
        owner.add_multistate_value(1, "Most", 1024)
        await owner.start()
        try:
            value = await owner.read_property(rb.ObjectIdentifier(O.ANALOG_INPUT, 1),
                                              P.PRESENT_VALUE)
            self.assertEqual(value.value, 1.5)
            states = await owner.read_property(rb.ObjectIdentifier(O.MULTI_STATE_VALUE, 1),
                                               P.NUMBER_OF_STATES)
            self.assertEqual(states.value, 1024)
            with self.assertRaises(RuntimeError):
                owner.add_analog_input(3, "Late")
        finally:
            await owner.stop()


if __name__ == "__main__":
    unittest.main()
