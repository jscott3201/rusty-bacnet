"""Complete Multiple COV preflight precedes future creation and client I/O."""
import asyncio
import socket
import unittest

from rusty_bacnet import BACnetClient, ObjectIdentifier, ObjectType, PropertyIdentifier

OID = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)


def reference(property_identifier=PropertyIdentifier.PRESENT_VALUE):
    return (property_identifier, None, None, False)


def valid_specs():
    return [(OID, [reference()])]


def invalid_specs():
    yield valid_specs() + [(OID, [])]
    for selector in (PropertyIdentifier.ALL, PropertyIdentifier.REQUIRED, PropertyIdentifier.OPTIONAL):
        yield valid_specs() + [(OID, [reference(), reference(selector)])]


class CovMultiplePreflightTests(unittest.IsolatedAsyncioTestCase):
    async def test_invalid_late_nested_reference_is_synchronous_before_address_and_start(self):
        client = BACnetClient(interface="127.0.0.1", port=0)
        for case, specs in enumerate(invalid_specs()):
            with self.subTest(case=case):
                try:
                    result = client.subscribe_cov_property_multiple(
                        "invalid-address", 1, specs, False, max_notification_delay=10, lifetime=300)
                except ValueError:
                    continue
                # Consume a baseline awaitable so the red does not leak a task.
                delayed_error = None
                try:
                    await result
                except Exception as error:
                    delayed_error = type(error).__name__
                self.fail(f"invalid nested input returned an awaitable; delayed error={delayed_error}")

    async def test_invalid_timing_and_nested_cap_are_synchronous(self):
        client = BACnetClient(interface="127.0.0.1", port=0)
        for lifetime, delay in ((None, 1), (60, None), (0, 0), (60, 60), (7200, 3601)):
            with self.subTest(lifetime=lifetime, delay=delay):
                with self.assertRaises(ValueError):
                    client.subscribe_cov_property_multiple(
                        "invalid-address", 1, valid_specs(), False,
                        max_notification_delay=delay, lifetime=lifetime)
        # The shared request caps cumulative references at 10,000, across specifications.
        with self.assertRaises(ValueError):
            client.subscribe_cov_property_multiple(
                "invalid-address", 1, [(OID, [reference()] * 10_000)] + valid_specs(), False,
                max_notification_delay=10, lifetime=300)

    async def test_started_invalid_later_spec_emits_no_prefix(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            address = f"127.0.0.1:{peer.getsockname()[1]}"
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                for specs in invalid_specs():
                    with self.assertRaises(ValueError):
                        client.subscribe_cov_property_multiple(
                            address, 1, specs, False, max_notification_delay=10, lifetime=300)
                with self.assertRaises(TimeoutError):
                    await asyncio.wait_for(asyncio.get_running_loop().sock_recvfrom(peer, 2048), .1)

    async def test_finite_and_whole_cancel_awaitables_send_exact_wire(self):
        loop = asyncio.get_running_loop()
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            address = f"127.0.0.1:{peer.getsockname()[1]}"
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                for specs, timing, expected in (
                    (valid_specs(), dict(lifetime=60, max_notification_delay=5),
                     bytes.fromhex("09 01 19 00 29 3c 39 05 4e 0c 00 00 00 01 1e 0e 09 55 0f 29 00 1f 4f")),
                    ([], {}, bytes.fromhex("09 01 19 00 4e 4f")),
                ):
                    operation = client.subscribe_cov_property_multiple(address, 1, specs, False, **timing)
                    self.assertTrue(hasattr(operation, "__await__"))
                    packet, sender = await asyncio.wait_for(loop.sock_recvfrom(peer, 2048), 2)
                    self.assertEqual(packet[:2], b"\x81\x0a")
                    self.assertEqual(packet[4:6], b"\x01\x04")
                    self.assertEqual(packet[6] & 0xf0, 0)
                    self.assertEqual(packet[9], 30)  # SubscribeCOVPropertyMultiple
                    self.assertEqual(packet[10:], expected)
                    reply = b"\x81\x0a\x00\x09\x01\x00\x20" + bytes((packet[8], 30))
                    await loop.sock_sendto(peer, reply, sender)
                    await asyncio.wait_for(operation, 2)
