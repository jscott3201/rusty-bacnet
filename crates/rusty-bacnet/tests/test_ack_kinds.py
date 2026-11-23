"""An acknowledgement of the other kind than the service takes (#2721).

A raw B/IP peer answers a WriteProperty with a Complex-ACK, and a
ReadProperty with a Simple-ACK. Each request fails at once with
BacnetError, not as a success, a timeout or a decoding error, and the
client sends the peer nothing more: no retry, no Abort.
"""
import asyncio
import unittest

import rusty_bacnet as rb
from bvll_peer import RawPeer, bvll

AI_0 = rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 0)
PV = rb.PropertyIdentifier.PRESENT_VALUE


def answering(kind):
    """A handler that acknowledges each confirmed request with `kind`."""
    def handler(datagram, source):
        if datagram[:2] != b"\x81\x0a":
            return None
        apdu = datagram[6:]  # after the BVLL header and a two-octet NPCI
        if apdu[0] >> 4 != 0:
            return None
        invoke, service = apdu[2], apdu[3]
        if kind == "complex":
            reply = bytes([0x30, invoke, service, 0x21, 0x2A])
        else:
            reply = bytes([0x20, invoke, service])
        return [bvll(0x0A, b"\x01\x00" + reply)]
    return handler


class AckKindTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        # A retry would go after 2 s, long after these calls end.
        self.client = rb.BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
        await self.client.__aenter__()
        self.addAsyncCleanup(self.stop)

    async def stop(self):
        # The native stop's future needs the running loop.
        await asyncio.wait_for(self.client.stop(), 10)

    def peer(self, kind):
        peer = RawPeer(answering(kind))
        self.addCleanup(peer.close)
        return peer

    async def fails_once(self, peer, call, service):
        """The call raises the wrong-kind error, which names the service
        (a timeout would be an abort naming none), and the peer hears
        nothing after the request."""
        with self.assertRaises(rb.BacnetError) as caught:
            await asyncio.wait_for(call, 10)
        error = caught.exception
        self.assertIs(type(error), rb.BacnetError)
        self.assertIn(service, str(error))
        await asyncio.sleep(0.3)
        self.assertEqual(len(peer.received), 1, "the client sent more")
        return error

    async def test_a_complex_ack_to_a_write_fails_it(self):
        peer = self.peer("complex")
        call = self.client.write_property(peer.address, AI_0, PV, rb.PropertyValue.real(1.5))
        error = await self.fails_once(peer, call, "write-property")
        self.assertIn("with a Complex-ACK", str(error))

    async def test_a_simple_ack_to_a_read_fails_it(self):
        peer = self.peer("simple")
        call = self.client.read_property(peer.address, AI_0, PV)
        error = await self.fails_once(peer, call, "read-property")
        self.assertIn("with a Simple-ACK", str(error))


class LegacyAckTests(unittest.IsolatedAsyncioTestCase):
    """``accept_complex_ack_for_simple_services=True``, for a device in the
    field that acknowledges writes with a Complex-ACK: the write succeeds;
    a Simple-ACK to a read is still an error."""

    async def asyncSetUp(self):
        self.client = rb.BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000,
                                      accept_complex_ack_for_simple_services=True)
        await self.client.__aenter__()
        self.addAsyncCleanup(self.stop)

    async def stop(self):
        await asyncio.wait_for(self.client.stop(), 10)

    async def test_the_write_succeeds_and_the_read_still_fails(self):
        writes = RawPeer(answering("complex"))
        self.addCleanup(writes.close)
        await asyncio.wait_for(
            self.client.write_property(writes.address, AI_0, PV, rb.PropertyValue.real(1.5)), 10)
        reads = RawPeer(answering("simple"))
        self.addCleanup(reads.close)
        with self.assertRaises(rb.BacnetError) as caught:
            await asyncio.wait_for(self.client.read_property(reads.address, AI_0, PV), 10)
        self.assertIn("read-property with a Simple-ACK", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
