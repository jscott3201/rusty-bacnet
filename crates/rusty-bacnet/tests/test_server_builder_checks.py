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
                with self.assertRaisesRegex(rb.BacnetError, "4194303|code=37"):
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


class PureChecksBeforeStorageTests(unittest.TestCase):
    """A check on a list keyword runs before the builder opens or clears its
    storage file, so a refusal leaves the file as it was."""

    def kept_file(self, name):
        path = Path(SCRATCH.name) / name
        path.write_bytes(b"not a storage file")
        return path

    def test_a_timer_priority_the_setter_refuses_clears_nothing(self):
        path = self.kept_file("timer-priority")
        owner = server()
        # The sentinel is no storage file, so opening it would refuse with
        # "no valid header": the refusal must be the setter's instead.
        with self.assertRaises(rb.BacnetError) as refused:
            owner.add_timer(
                1, "Timer", state_change_values=[rb.PropertyValue.unsigned(1)] * 7,
                priority_for_writing=0, storage_path=str(path),
                clear_written_config=True)
        self.assertNotIn("no valid header", str(refused.exception))
        self.assertEqual(path.read_bytes(), b"not a storage file")
        self.assertEqual(owner._pending_registration_count(), 0)

    def test_too_many_forwarder_recipients_open_nothing(self):
        path = self.kept_file("forwarder-recipients")
        device = rb.ObjectIdentifier(rb.ObjectType.DEVICE, 99)
        recipients = [{"recipient": {"kind": "device", "object_identifier": device}, "process_identifier": n} for n in range(1, 34)]
        owner = server()
        with self.assertRaises(Exception) as refused:
            owner.add_notification_forwarder(
                1, "Forwarder", storage_path=str(path), recipients=recipients)
        self.assertNotIn("no valid header", str(refused.exception))
        self.assertEqual(path.read_bytes(), b"not a storage file")
        self.assertEqual(owner._pending_registration_count(), 0)


class RegisteredPortStartTests(unittest.IsolatedAsyncioTestCase):
    """A registered_network_port the Rust server's builder would refuse is
    refused by start() while the registrations are still there."""

    def node(self):
        owner = rb.BACnetServer(
            2748, device_name=DEVICE_NAME, interface="127.0.0.1", port=0,
            registered_network_port=1)
        owner.add_file(1, "Kept file")
        return owner

    async def test_a_port_that_was_never_added_is_added_and_the_start_serves(self):
        owner = self.node()
        for _ in range(2):
            with self.assertRaisesRegex(ValueError, "names no Network Port"):
                await owner.start()
            self.assertEqual(owner._pending_registration_count(), 1)
        owner.add_bip_network_port(1, "Port", ip_address="127.0.0.1", udp_port=0)
        await owner.start()
        try:
            self.assertEqual(owner._pending_registration_count(), 0)
            name = await owner.read_property(
                rb.ObjectIdentifier(rb.ObjectType.FILE, 1), rb.PropertyIdentifier.OBJECT_NAME)
            self.assertEqual(name.value, "Kept file")
        finally:
            await owner.stop()

    async def test_a_port_that_differs_from_the_transport_is_refused_with_the_rest_kept(self):
        cases = (
            ("the wrong family", lambda o: o.add_bip6_network_port(1, "Port"), "not a B/IP port"),
            ("another address", lambda o: o.add_bip_network_port(
                1, "Port", ip_address="127.0.0.2", udp_port=0), "differs"),
            ("another UDP port", lambda o: o.add_bip_network_port(
                1, "Port", ip_address="127.0.0.1", udp_port=47808), "differs"),
            ("an unspecified address", lambda o: o.add_bip_network_port(1, "Port"), "differs"),
        )
        for label, add, message in cases:
            with self.subTest(label):
                owner = self.node()
                add(owner)
                with self.assertRaisesRegex(ValueError, message):
                    await owner.start()
                self.assertEqual(owner._pending_registration_count(), 2)


class StartStateTests(unittest.IsolatedAsyncioTestCase):
    async def test_a_second_start_of_a_running_server_is_refused(self):
        owner = server()
        owner.add_file(1, "File")
        await owner.start()
        try:
            with self.assertRaisesRegex(RuntimeError, "already running"):
                owner.start()
            with self.assertRaisesRegex(RuntimeError, "after start"):
                owner.add_file(2, "Late")
        finally:
            await owner.stop()
        # Stopped, it may start again.
        await owner.start()
        await owner.stop()


if __name__ == "__main__":
    unittest.main()
