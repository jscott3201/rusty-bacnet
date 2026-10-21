"""Installed full-server stop retains a cancelled drain and releases its UDP socket."""
import asyncio
import socket
import unittest

import rusty_bacnet as rb
from port_retry import rerun_on_lost_port


class ServerShutdownTests(unittest.IsolatedAsyncioTestCase):
    async def test_cancelled_stop_can_rejoin_confirmed_audit_drain(self):
        # The closing rebind can lose the freed port to another process (#1419).
        await rerun_on_lost_port(self.stop_drain_and_release)

    async def stop_drain_and_release(self):
        loop = asyncio.get_running_loop()
        server = rb.BACnetServer(8721, interface="127.0.0.1", port=0)
        server.add_binary_value(1, "shutdown-value")
        server.add_audit_reporter(1, "shutdown-reporter")
        server.configure_audit_reporters([{
            "instance": 1, "audit_level": "audit_all", "auditable_operations": 2,
            "issue_confirmed_notifications": True, "maximum_send_delay": 3600,
        }])
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as recipient:
            recipient.bind(("127.0.0.1", 0))
            recipient.setblocking(False)
            server.add_device_binding(8722, f"127.0.0.1:{recipient.getsockname()[1]}")
            server.configure_audit_recipient({
                "kind": "device",
                "object_identifier": rb.ObjectIdentifier(rb.ObjectType.DEVICE, 8722),
            })
            await server.start()
            address = await server.local_address()
            port = int(address.rsplit(":", 1)[1])
            pending = []
            try:
                async with rb.BACnetClient(interface="127.0.0.1", port=0) as client:
                    await client.write_property(
                        address, rb.ObjectIdentifier(rb.ObjectType.BINARY_VALUE, 1),
                        rb.PropertyIdentifier.PRESENT_VALUE, rb.PropertyValue.enumerated(1),
                    )
                # Only stop flushes this delayed batch. Its captured request
                # proves Rust has entered the drain while retaining the owner lock.
                first = asyncio.ensure_future(server.stop())
                pending.append(first)
                wire, remote = await asyncio.wait_for(loop.sock_recvfrom(recipient, 4096), 2)
                self.assertEqual(wire[:2], b"\x81\x0a")
                self.assertEqual(wire[9], 32)  # ConfirmedAuditNotification.
                self.assertEqual(wire[4], 1)
                self.assertEqual(wire[6] & 0xf0, 0)  # Confirmed request, direct NPDU.
                # The outstanding wire-confirmed notification holds the drain.
                # A second call shares its owner; it must not complete early.
                observer = asyncio.ensure_future(server.local_address())
                pending.append(observer)
                done, _ = await asyncio.wait([first, observer], timeout=0.05)
                self.assertEqual(done, set(), "stop must retain owner lock during ACK drain")
                first.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await first
                # Synchronize with actual Rust cancellation and mutex release.
                self.assertEqual(await asyncio.wait_for(observer, 2), address)
                second = asyncio.ensure_future(server.stop())
                pending.append(second)
                done, _ = await asyncio.wait([second], timeout=0.05)
                self.assertEqual(done, set(), "cancelled stop lost its cleanup owner")
                payload = b"\x01\x00\x20" + bytes([wire[8], wire[9]])
                await loop.sock_sendto(recipient, b"\x81\x0a\x00\x09" + payload, remote)
                self.assertIsNone(await asyncio.wait_for(second, 2))
                self.assertIsNone(await server.stop())
                with self.assertRaises(RuntimeError):
                    await server.local_address()
                with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as rebound:
                    rebound.bind(("0.0.0.0", port))
            finally:
                for task in pending:
                    if not task.done():
                        task.cancel()
                await asyncio.gather(*pending, return_exceptions=True)
                await server.stop()
