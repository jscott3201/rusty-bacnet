"""Every ``BACnetServer.add_*`` builder checks identifiers and names at the
call (#2748), as the nine point builders do (#2711): a repeated identifier or
name raises ValueError, instance 4194303 raises BacnetError, and a refused
call keeps nothing and touches no storage file. The cases are one table, and
a test fails for a builder the table lacks, so a new builder cannot skip them.
"""
import inspect
import itertools
import tempfile
import unittest
from pathlib import Path

import rusty_bacnet as rb

RESERVED = 4194303
DEVICE_NAME = "Host"
SOURCE = rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 1)
STAGES = [(10.0, [False, True], 1.0), (20.0, [True, True], 2.0)]
TARGETS = [
    rb.ObjectIdentifier(rb.ObjectType.BINARY_OUTPUT, 1),
    rb.ObjectIdentifier(rb.ObjectType.BINARY_VALUE, 1),
]

# Builders that take more than an instance and a name, and what they take.
EXTRA = {
    "add_multistate_input": (3,),
    "add_multistate_output": (3,),
    "add_multistate_value": (3,),
    "add_channel": (1,),
    "add_alert_enrollment": (SOURCE,),
    "add_lift": (2,),
    "add_staging": (5.0, 0.0, 62, 8, STAGES, TARGETS),
}

# `add_*` methods that register no object.
NOT_OBJECT_BUILDERS = {
    "add_device_binding",
    "add_averaging_sample_local",
    "add_pulses_local",
}

# Every other `add_*` method, found on the class: one the table cannot call
# fails `test_every_builder_has_a_default_call`, so none is skipped.
BUILDERS = sorted(
    name for name in dir(rb.BACnetServer)
    if name.startswith("add_") and name not in NOT_OBJECT_BUILDERS
)

SCRATCH = None
UNIQUE = itertools.count()


def setUpModule():
    global SCRATCH
    SCRATCH = tempfile.TemporaryDirectory()


def tearDownModule():
    SCRATCH.cleanup()


def server():
    return rb.BACnetServer(
        2748, device_name=DEVICE_NAME, interface="127.0.0.1", port=0
    )


def call(owner, builder, instance, name, **keywords):
    """One call of `builder`. Only `add_audit_log` must be given a storage
    path, so it gets a fresh one."""
    if builder == "add_audit_log" and "storage_path" not in keywords:
        keywords["storage_path"] = str(Path(SCRATCH.name) / f"audit-{next(UNIQUE)}")
    return getattr(owner, builder)(
        instance, name, *EXTRA.get(builder, ()), **keywords
    )


def clearing_keywords(builder):
    """For a builder that keeps a storage file, the keywords that ask it to
    clear what the file holds; None for one that keeps none."""
    parameters = inspect.signature(getattr(rb.BACnetServer, builder)).parameters
    if "storage_path" not in parameters:
        return None
    return [name for name in parameters if name.startswith("clear_written")]


class BuilderTableTests(unittest.TestCase):
    def test_the_exceptions_to_the_table_are_real_methods(self):
        known = set(dir(rb.BACnetServer))
        self.assertLessEqual(set(EXTRA) | NOT_OBJECT_BUILDERS, known)
        self.assertGreaterEqual(len(BUILDERS), 60)

    def test_every_builder_has_a_default_call(self):
        for builder in BUILDERS:
            with self.subTest(builder):
                owner = server()
                call(owner, builder, 1, "Probe")
                self.assertEqual(owner._pending_registration_count(), 1)


class ChecksAtTheCallTests(unittest.TestCase):
    def test_a_repeated_identifier_is_refused_and_the_first_kept(self):
        for builder in BUILDERS:
            with self.subTest(builder):
                owner = server()
                call(owner, builder, 1, "First")
                with self.assertRaisesRegex(ValueError, "^duplicate object identifier"):
                    call(owner, builder, 1, "Second")
                self.assertEqual(owner._pending_registration_count(), 1)

    def test_a_repeated_name_is_refused(self):
        for builder in BUILDERS:
            with self.subTest(builder):
                owner = server()
                call(owner, builder, 1, "Taken")
                with self.assertRaisesRegex(ValueError, "^duplicate object name: Taken"):
                    call(owner, builder, 2, "Taken")
                self.assertEqual(owner._pending_registration_count(), 1)

    def test_the_devices_name_is_taken(self):
        for builder in BUILDERS:
            with self.subTest(builder):
                owner = server()
                with self.assertRaisesRegex(
                    ValueError, f"^duplicate object name: {DEVICE_NAME}"
                ):
                    call(owner, builder, 1, DEVICE_NAME)
                self.assertEqual(owner._pending_registration_count(), 0)

    def test_the_reserved_instance_is_refused(self):
        for builder in BUILDERS:
            with self.subTest(builder):
                owner = server()
                # BacnetError, or its BacnetProtocolError subclass from an
                # object whose own constructor refuses the instance first.
                with self.assertRaises(rb.BacnetError):
                    call(owner, builder, RESERVED, "Reserved")
                self.assertEqual(owner._pending_registration_count(), 0)

    def test_only_the_same_type_clashes_on_an_instance(self):
        owner = server()
        kept = 0
        for builder in BUILDERS:
            with self.subTest(builder):
                # Instance 1 of each type under its own name: several
                # builders make one type, and only those clash.
                try:
                    call(owner, builder, 1, "N-" + builder)
                except ValueError as error:
                    self.assertIn("duplicate object identifier", str(error))
                else:
                    kept += 1
        self.assertEqual(owner._pending_registration_count(), kept)
        self.assertGreater(kept, 40)

    def test_a_refused_call_touches_no_storage_file(self):
        with_storage = [b for b in BUILDERS if clearing_keywords(b) is not None]
        self.assertGreaterEqual(len(with_storage), 15)
        for builder in with_storage:
            clears = {name: True for name in clearing_keywords(builder)}
            with self.subTest(builder):
                # Not a storage file: a build would refuse it, naming the
                # path, and a clear would overwrite it.
                path = Path(SCRATCH.name) / f"kept-{builder}"
                path.write_bytes(b"not a storage file")
                owner = server()
                call(owner, builder, 1, "First")
                for instance, name, error in (
                    (1, "Second", ValueError),
                    (2, DEVICE_NAME, ValueError),
                    (RESERVED, "Reserved", rb.BacnetError),
                ):
                    with self.assertRaises(error):
                        call(owner, builder, instance, name,
                             storage_path=str(path), **clears)
                self.assertEqual(path.read_bytes(), b"not a storage file")
                self.assertEqual(owner._pending_registration_count(), 1)


class StartStateTests(unittest.IsolatedAsyncioTestCase):
    async def test_registrations_during_a_start_and_a_second_start_are_refused(self):
        owner = server()
        owner.add_file(1, "File")
        starting = owner.start()
        with self.assertRaisesRegex(RuntimeError, "in progress"):
            owner.add_file(2, "Late")
        with self.assertRaisesRegex(RuntimeError, "already in progress"):
            owner.start()
        await starting
        try:
            with self.assertRaisesRegex(RuntimeError, "after start"):
                owner.add_file(2, "Late")
            self.assertEqual(owner._pending_registration_count(), 0)
        finally:
            await owner.stop()


if __name__ == "__main__":
    unittest.main()
