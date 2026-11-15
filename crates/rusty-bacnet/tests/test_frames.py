"""Raw transport observation beside both endpoint roles, without another stack socket."""
import asyncio
import gc
import socket
import unittest

from rusty_bacnet import BACnetClient, BipEndpoint, Frame, FrameIterator
from rusty_bacnet import ObjectIdentifier, ObjectType, PropertyIdentifier, PropertyValue


class FrameTests(unittest.IsolatedAsyncioTestCase):
    async def endpoint(self, instance=710):
        endpoint = BipEndpoint(instance, interface="127.0.0.1", port=0)
        await endpoint.start()
        async def close():
            await endpoint.close()
        self.addAsyncCleanup(close)
        return endpoint

    async def next(self, frames):
        # Outer liveness guard, never a timing assertion.
        return await asyncio.wait_for(anext(frames), 10)

    def peer(self):
        peer = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        peer.bind(("127.0.0.1", 0))
        peer.setblocking(False)
        self.addCleanup(peer.close)
        return peer

    async def send(self, peer, address, payload):
        host, port = address.rsplit(":", 1)
        await asyncio.get_running_loop().sock_sendto(peer, payload, (host, int(port)))

    async def test_both_roles_work_on_the_observed_socket(self):
        endpoint = await self.endpoint()
        remote = await self.endpoint(711)
        frames = await endpoint.frames()
        self.assertIsInstance(frames, FrameIterator)
        remote_frames = await remote.frames()
        client = await endpoint.client()
        other = await remote.client()
        oid = ObjectIdentifier(ObjectType.DEVICE, 711)
        own_oid = ObjectIdentifier(ObjectType.DEVICE, 710)
        value, own_value = await asyncio.gather(
            client.read_property(await remote.local_address(), oid, PropertyIdentifier.OBJECT_IDENTIFIER),
            other.read_property(await endpoint.local_address(), own_oid, PropertyIdentifier.OBJECT_IDENTIFIER),
        )
        self.assertEqual(value, PropertyValue.object_identifier(oid))
        self.assertEqual(own_value, PropertyValue.object_identifier(own_oid))
        observed = [await self.next(frames) for _ in range(4)]
        others = [await self.next(remote_frames) for _ in range(4)]
        self.assertEqual({frame.direction for frame in observed}, {"inbound", "outbound"})
        self.assertEqual(sorted(frame.octets for frame in observed), sorted(frame.octets for frame in others))
        self.assertEqual([f.timestamp for f in observed], sorted(f.timestamp for f in observed))
        for frame in observed:
            self.assertIsInstance(frame, Frame)
            self.assertEqual(frame.link, "BACnet/IP")
            self.assertEqual(frame.local_address, await endpoint.local_address())
            self.assertEqual(frame.peer, await remote.local_address())
            self.assertEqual(frame.missed, 0)
            with self.assertRaises(AttributeError):
                frame.timestamp = 0

    async def test_malformed_datagrams_and_exact_overflow_count(self):
        endpoint = await self.endpoint()
        address = await endpoint.local_address()
        peer = self.peer()
        slow = await endpoint.frames(capacity=1)
        witness = await endpoint.frames(capacity=8)
        for index in range(5):
            payload = b"malformed datagram " + bytes([index])
            await self.send(peer, address, payload)
            event = await self.next(witness)  # Fence the actual transport receipt.
            self.assertEqual(event.octets, payload)
            self.assertEqual(event.peer, "%s:%d" % peer.getsockname())
            self.assertEqual(event.direction, "inbound")
        self.assertEqual(slow.missed, 4)
        first = await self.next(slow)
        self.assertEqual(first.octets, b"malformed datagram \x00")
        self.assertEqual(first.missed, 4)
        self.assertEqual(witness.missed, 0)
        await slow.aclose()
        self.assertEqual(slow.missed, 4)
        with self.assertRaises(StopAsyncIteration):
            await self.next(slow)

    async def test_cancelled_next_keeps_subscription_and_next_frame(self):
        endpoint = await self.endpoint()
        frames = await endpoint.frames()
        pending = asyncio.ensure_future(anext(frames))
        pending.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await pending
        payload = b"after cancelled wait"
        await self.send(self.peer(), await endpoint.local_address(), payload)
        self.assertEqual((await self.next(frames)).octets, payload)
        self.assertEqual(frames.missed, 0)

    async def test_close_wakes_multiple_pending_reads_and_is_idempotent(self):
        endpoint = await self.endpoint()
        frames = await endpoint.frames()
        pending = [asyncio.ensure_future(anext(frames)) for _ in range(3)]
        await frames.aclose()
        await frames.aclose()
        for waiter in pending:
            with self.assertRaises(StopAsyncIteration):
                await asyncio.wait_for(waiter, 10)
        # Observation close leaves both protocol roles alive.
        self.assertTrue((await endpoint.server()).is_session_alive())

    async def test_endpoint_close_and_restart_keep_generations_separate(self):
        endpoint = await self.endpoint()
        frames = await endpoint.frames()
        pending = asyncio.ensure_future(anext(frames))
        await endpoint.close()
        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(pending, 10)
        with self.assertRaises(RuntimeError):
            await endpoint.frames()
        await endpoint.start()
        fresh = await endpoint.frames()
        payload = b"new generation"
        await self.send(self.peer(), await endpoint.local_address(), payload)
        self.assertEqual((await self.next(fresh)).octets, payload)
        with self.assertRaises(StopAsyncIteration):
            await self.next(frames)

    async def test_owner_drop_wakes_observer_without_retaining_socket(self):
        endpoint = BipEndpoint(713, interface="127.0.0.1", port=0)
        await endpoint.start()
        frames = await endpoint.frames()
        pending = asyncio.ensure_future(anext(frames))
        del endpoint
        gc.collect()
        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(pending, 10)

    async def test_standalone_client_stop_closes_iterator(self):
        endpoint = await self.endpoint()
        async with BACnetClient(interface="127.0.0.1", port=0) as client:
            frames = await client.frames()
            oid = ObjectIdentifier(ObjectType.DEVICE, 710)
            await client.read_property(await endpoint.local_address(), oid, PropertyIdentifier.OBJECT_IDENTIFIER)
            outgoing, incoming = await self.next(frames), await self.next(frames)
            self.assertEqual((outgoing.direction, incoming.direction), ("outbound", "inbound"))
            self.assertLessEqual(outgoing.timestamp, incoming.timestamp)
            pending = asyncio.ensure_future(anext(frames))
        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(pending, 10)
        with self.assertRaises(RuntimeError):
            await client.frames()

    async def test_configuration_and_inactive_owners_refuse(self):
        endpoint = BipEndpoint(714, interface="127.0.0.1", port=0)
        client = BACnetClient(interface="127.0.0.1", port=0)
        for owner in (endpoint, client):
            for capacity in (0, 65537):
                with self.assertRaises(ValueError):
                    owner.frames(capacity=capacity)
            with self.assertRaises(RuntimeError):
                await owner.frames()


if __name__ == "__main__":
    unittest.main()
