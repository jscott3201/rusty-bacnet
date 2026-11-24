"""A start the Rust server refuses keeps the registrations (#2752).

The server's builder hands its database back when it refuses, so a port
another program holds, the most common B/IP start failure, costs nothing:
every registration, the Audit recipient and the forwarders' save counters
are there for the next start, which serves every object once the port is
free.
"""
import asyncio
import socket
import unittest

import rusty_bacnet as rb
from port_retry import rerun_on_lost_port

REPORTER = {"instance": 1, "audit_level": "audit_all", "auditable_operations": 2,
            "issue_confirmed_notifications": False}
OBJECTS = [
    (rb.ObjectType.ANALOG_INPUT, 0, "Kept AI"),
    (rb.ObjectType.FILE, 7, "Kept file"),
    (rb.ObjectType.AUDIT_REPORTER, 1, "Kept reporter"),
    (rb.ObjectType.NOTIFICATION_FORWARDER, 3, "Kept forwarder"),
]


def holder():
    """A plain socket on a port the OS picks, as another program would hold
    it: on the wildcard address, which a claiming owner on 127.0.0.1 is
    refused beside on every platform (test_share_wildcard_port)."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("0.0.0.0", 0))
    return sock


def kept_node(port):
    """A B/IP server on 127.0.0.1:`port` with four registrations, an Audit
    recipient and the Reporter selected."""
    node = rb.BACnetServer(2752, device_name="Kept device", interface="127.0.0.1", port=port)
    node.add_analog_input(0, "Kept AI", 64, 72.5)
    node.add_file(7, "Kept file")
    node.add_audit_reporter(1, "Kept reporter")
    node.add_notification_forwarder(3, "Kept forwarder")
    node.configure_audit_recipient({
        "kind": "device",
        "object_identifier": rb.ObjectIdentifier(rb.ObjectType.DEVICE, 9),
    })
    node.configure_audit_reporters([REPORTER])
    return node


class RefusedBindTests(unittest.IsolatedAsyncioTestCase):
    async def test_a_port_in_use_keeps_every_registration_for_the_next_start(self):
        async def run():
            with holder() as held:
                port = held.getsockname()[1]
                node = kept_node(port)
                for _ in range(2):
                    with self.assertRaises(rb.BacnetTransportError) as refused:
                        await asyncio.wait_for(node.start(), 10)
                    self.assertIn("share_wildcard_port", str(refused.exception))
                    self.assertEqual(node._pending_registration_count(), len(OBJECTS))
                    # Not frozen: the Reporter is selected again.
                    node.configure_audit_reporters([REPORTER])
                # The registrations came back whole: their names and
                # identifiers are taken.
                with self.assertRaisesRegex(ValueError, "duplicate object name: Kept AI"):
                    node.add_binary_input(5, "Kept AI")
                with self.assertRaises(ValueError):
                    node.add_file(7, "Another file")
                node.add_binary_input(5, "Added after the refusal")
            # The port is free: another process can take it, which reruns.
            await asyncio.wait_for(node.start(), 10)
            try:
                self.assertEqual(node._pending_registration_count(), 0)
                for object_type, instance, name in OBJECTS + [
                        (rb.ObjectType.BINARY_INPUT, 5, "Added after the refusal")]:
                    read = await node.read_property(
                        rb.ObjectIdentifier(object_type, instance),
                        rb.PropertyIdentifier.OBJECT_NAME)
                    self.assertEqual(read.value, name)
                # The forwarder's save counters came back with it.
                self.assertIn(3, await node.forwarder_save_counters())
                self.assertEqual(await node.local_address(), f"127.0.0.1:{port}")
            finally:
                await node.stop()

        await rerun_on_lost_port(run)


if __name__ == "__main__":
    unittest.main()
