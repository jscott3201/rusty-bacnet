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
        # Two faults in one call: each surface reports the same one first.
        ("a taken name and a NaN", lambda o: o.add_analog_input(5, "Kept", 62, math.nan)),
        ("the reserved instance and a taken name",
         lambda o: o.add_binary_value(4194303, "Kept")),
        ("a taken identifier and a refused profile",
         lambda o: o.add_analog_input(1, "Other", profile_name="room")),
        ("a refused profile and a NaN",
         lambda o: o.add_analog_input(5, "Bad", 62, math.nan, profile_name="room")),
        ("a refused profile and too many states",
         lambda o: o.add_multistate_input(5, "Bad", 1025, profile_name="room")),
    ]
    # The error each two-fault call must give: the first in admit's order.
    FIRST = {
        "a taken name and a NaN": (ValueError, "duplicate object name"),
        "the reserved instance and a taken name": (rb.BacnetError, "4194303"),
        "a taken identifier and a refused profile": (ValueError, "duplicate object identifier"),
        "a refused profile and a NaN": (rb.BacnetProtocolError, ""),
        "a refused profile and too many states": (rb.BacnetProtocolError, ""),
    }

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
                if case in self.FIRST:
                    error, text = self.FIRST[case]
                    kind, message = by_owner["server"]
                    self.assertTrue(issubclass(kind, error), by_owner)
                    self.assertIn(text, message)
        # A profile refusal is the profile's, not a range error of the count.
        kind, message = results["a refused profile and too many states"]["server"]
        self.assertNotIn("1024", message)

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


class RefusedClearingRegistrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_a_refused_profile_leaves_a_kept_file_as_it_was(self):
        # A registration that would clear the file, refused for its profile:
        # the check comes before storage attaches, so nothing is cleared.
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / "av-1")
            oid = rb.ObjectIdentifier(O.ANALOG_VALUE, 1)
            first = server()
            first.add_analog_value(1, "AV", storage_path=path)
            await first.start()
            try:
                await first.write_property_local(
                    oid, P.DESCRIPTION, rb.PropertyValue.character_string("Lobby"),
                    source_object=None)
            finally:
                await first.stop()
            kept = Path(path).read_bytes()
            second = server()
            with self.assertRaises(rb.BacnetProtocolError) as raised:
                second.add_analog_value(1, "AV", storage_path=path,
                                        clear_written_reporting=True, profile_name="room")
            self.assertEqual(raised.exception.error_code,
                             rb.ErrorCode.VALUE_OUT_OF_RANGE.to_raw())
            self.assertEqual(Path(path).read_bytes(), kept)
            self.assertEqual(second._pending_registration_count(), 0)
            # The same call with a good profile clears it.
            second.add_analog_value(1, "AV", storage_path=path,
                                    clear_written_reporting=True, profile_name="555-room")
            await second.start()
            try:
                description = await second.read_property(oid, P.DESCRIPTION)
                self.assertEqual(description.value, "")
            finally:
                await second.stop()


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
