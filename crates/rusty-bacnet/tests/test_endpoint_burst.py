"""A burst of unsolicited traffic doesn't end a B/IP endpoint (#2635).

Before #2635 about a hundred I-Ams sent in a burst filled the endpoint's
request queue and then its policy queue, and the endpoint stopped:
`status()` raised "endpoint is not active". Now what a full queue can't take
is dropped and counted in `status()`, and the endpoint keeps answering.
"""

from __future__ import annotations

import asyncio
import socket
import struct
import unittest

from rusty_bacnet import BipEndpoint

FULL_DROP_KEYS = (
    "unconfirmed_full_drops",
    "confirmed_full_drops",
    "answer_full_drops",
    "policy_full_drops",
)


def bvll(npdu: bytes) -> bytes:
    """`npdu` in an Original-Unicast-NPDU."""
    return struct.pack("!BBH", 0x81, 0x0A, 4 + len(npdu)) + npdu


def i_am(instance: int) -> bytes:
    """The I-Am of device `instance` (Max_APDU 480, no segmentation, vendor 42)."""
    object_id = (8 << 22) | instance
    apdu = b"\x10\x00\xc4" + struct.pack("!I", object_id) + b"\x22\x01\xe0\x91\x03\x21\x2a"
    return bvll(b"\x01\x00" + apdu)


def read_present_value(invoke_id: int, instance: int) -> bytes:
    """A ReadProperty of Analog Input `instance`'s Present_Value."""
    object_id = (0 << 22) | instance
    apdu = (
        bytes([0x00, 0x05, invoke_id, 0x0C, 0x0C])
        + struct.pack("!I", object_id)
        + b"\x19\x55"
    )
    return bvll(b"\x01\x04" + apdu)


class EndpointBurstTests(unittest.IsolatedAsyncioTestCase):
    async def test_three_hundred_unsolicited_i_ams_leave_the_endpoint_running(self):
        # Four slots per queue: the burst overflows them at once.
        endpoint = BipEndpoint(
            device_instance=1001,
            vendor_id=42,
            interface="127.0.0.1",
            broadcast_address="127.255.255.255",
            port=0,
            queue_capacity=4,
        )
        endpoint.add_analog_input(instance=1, name="Zone", present_value=21.5)
        async with endpoint:
            address = await endpoint.local_address()
            port = int(address.rsplit(":", 1)[1])
            target = ("127.0.0.1", port)
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
                peer.bind(("127.0.0.1", 0))
                peer.settimeout(5)
                for n in range(300):
                    peer.sendto(i_am(20_000 + n), target)
                await asyncio.sleep(0.5)

                status = await endpoint.status()
                self.assertTrue(status["is_running"])
                for key in FULL_DROP_KEYS:
                    self.assertIsInstance(status[key], int, key)
                    self.assertGreaterEqual(status[key], 0, key)
                self.assertEqual(status["confirmed_full_drops"], 0)

                # The device still answers a read.
                peer.sendto(read_present_value(7, 1), target)
                reply = await asyncio.to_thread(self.receive_complex_ack, peer, 7)
                self.assertEqual(reply[:2], b"\x30\x07")
                self.assertTrue((await endpoint.status())["is_running"])

    @staticmethod
    def receive_complex_ack(peer: socket.socket, invoke_id: int) -> bytes:
        """The APDU of the first Complex-ACK for `invoke_id` the peer receives."""
        while True:
            frame, _ = peer.recvfrom(1500)
            # BVLL (4 octets), then an NPDU with no addresses (2 octets).
            apdu = frame[6:]
            if len(apdu) >= 2 and apdu[0] == 0x30 and apdu[1] == invoke_id:
                return apdu


if __name__ == "__main__":
    unittest.main()
