"""Concurrent Python client operations retain shutdown ownership across cancellation."""
import asyncio

from rusty_bacnet import ObjectIdentifier, ObjectType, PropertyIdentifier, ScReconnect
from sc_client_fixture import ClientScFixture


class ClientLifecycleTests(ClientScFixture):
    async def wait_for_closing(self, client):
        async def observed():
            while True:
                try:
                    await client.discovered_devices()
                except RuntimeError as error:
                    self.assertIn("closing", str(error))
                    return
                await asyncio.sleep(0)
        await asyncio.wait_for(observed(), 3)

    async def test_cancelled_close_drains_concurrent_operations_and_joins_held_retry(self):
        hub, _ = await self.serving_hub()
        relay = await self.relay(await hub.url())
        client = self.client(relay.url, apdu_timeout_ms=5_000, reconnect=ScReconnect(
            initial_delay_ms=100, max_delay_ms=100, max_retries=10))
        await asyncio.wait_for(client.__aenter__(), 5)
        await relay.next_connection()
        raw_reader, raw_writer = await self.incumbent(
            await hub.address(), bytes.fromhex("1f5099abc54846c78df3188ce0763781"))
        self.addAsyncCleanup(self.close_raw, raw_writer)
        requests = [asyncio.ensure_future(client.read_property(
            "02:00:00:00:00:03", ObjectIdentifier(ObjectType.ANALOG_INPUT, instance),
            PropertyIdentifier.PRESENT_VALUE)) for instance in [41, 42]]
        closing = joining = None
        try:
            # Both requests reach the peer before either is answered. A single
            # operation mutex would prevent this second independent wire event.
            observed = set()
            for _ in range(8):
                _, packet = await self.frame(raw_reader, False)
                for instance in [41, 42]:
                    if packet.endswith(bytes([12, 12, 0, 0, 0, instance, 25, 85])):
                        observed.add(instance)
                if observed == {41, 42}:
                    break
            self.assertEqual(observed, {41, 42})
            self.assertTrue(all(not request.done() for request in requests))

            relay.pause()
            await relay.disconnect()
            retry = await relay.next_connection()
            closing = asyncio.ensure_future(client.stop())
            await self.wait_for_closing(client)
            self.assertFalse(closing.done(), "close must drain the retained request leases")
            joining = asyncio.ensure_future(client.stop())
            closing.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await closing
            self.assertFalse(joining.done())
            with self.assertRaisesRegex(RuntimeError, "closing"):
                await client.__aenter__()
            for request in requests:
                request.cancel()
            for request in requests:
                with self.assertRaises(asyncio.CancelledError):
                    await request
            self.assertIsNone(await asyncio.wait_for(joining, 5))
            await self.socket_eof(retry)
            with self.assertRaises(asyncio.TimeoutError):
                await asyncio.wait_for(relay.accepted.get(), 0.35)
            with self.assertRaisesRegex(RuntimeError, "not started"):
                await client.discovered_devices()
            self.assertIsNone(await client.stop())
        finally:
            for request in requests:
                request.cancel()
            await asyncio.gather(*requests, return_exceptions=True)
            await client.stop()
            await asyncio.gather(*[f for f in [closing, joining] if f is not None],
                                 return_exceptions=True)

    async def held_entry(self, cancel_entry):
        hub, _ = await self.serving_hub()
        relay = await self.relay(await hub.url())
        relay.pause()
        client = self.client(relay.url, reconnect=ScReconnect(
            initial_delay_ms=100, max_delay_ms=100, max_retries=2))
        entering = asyncio.ensure_future(client.__aenter__())
        connection = await relay.next_connection()
        # Started before stop(): its native task may queue behind the held
        # entry before the close is admitted. It must still see the close at
        # once rather than wait for the entry (#2030).
        early = asyncio.ensure_future(client.discovered_devices())
        closing = asyncio.ensure_future(client.stop())
        try:
            with self.assertRaisesRegex(RuntimeError, "closing"):
                await asyncio.wait_for(early, 3)
            await self.wait_for_closing(client)
            self.assertFalse(closing.done(), "close waits for exclusive pending entry")
            if cancel_entry:
                entering.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await entering
                await self.socket_eof(connection)
            else:
                relay.resume()
                self.assertIs(await asyncio.wait_for(entering, 5), client)
            self.assertIsNone(await asyncio.wait_for(closing, 5))
            with self.assertRaisesRegex(RuntimeError, "not started"):
                await client.discovered_devices()
            relay.resume()
            self.assertIs(await asyncio.wait_for(client.__aenter__(), 5), client)
            await self.read_value(client)
        finally:
            entering.cancel()
            early.cancel()
            await asyncio.gather(entering, early, return_exceptions=True)
            await client.stop()
            await asyncio.gather(closing, return_exceptions=True)

    async def test_cancelled_pending_entry_closes_socket_and_allows_later_entry(self):
        await self.held_entry(cancel_entry=True)

    async def test_stop_waits_for_pending_entry_then_closes_published_client(self):
        await self.held_entry(cancel_entry=False)

    async def test_concurrent_and_repeated_entry_cannot_replace_live_client(self):
        hub, _ = await self.serving_hub()
        relay = await self.relay(await hub.url())
        relay.pause()
        client = self.client(relay.url)
        first = asyncio.ensure_future(client.__aenter__())
        await relay.next_connection()
        second = asyncio.ensure_future(client.__aenter__())
        try:
            relay.resume()
            self.assertIs(await asyncio.wait_for(first, 5), client)
            with self.assertRaisesRegex(RuntimeError, "already started"):
                await asyncio.wait_for(second, 5)
            with self.assertRaisesRegex(RuntimeError, "already started"):
                await client.__aenter__()
            self.assertEqual(len(relay.connections), 1)
            await self.read_value(client)
        finally:
            await asyncio.gather(first, second, return_exceptions=True)
            await client.stop()

    async def test_cancelled_sc_admission_drops_socket_and_can_enter_again(self):
        requests = asyncio.Queue()
        release = asyncio.Event()
        closed = asyncio.Event()
        count = 0

        async def peer(reader, writer, connect):
            nonlocal count
            count += 1
            requests.put_nowait(connect)
            if count == 1:
                # TLS and WebSocket completed; SC Connect-Accept is withheld.
                self.assertEqual(await asyncio.wait_for(reader.read(1), 3), b"")
                closed.set()
                return
            await self.accept_sc(writer, connect)
            await release.wait()

        client = self.client(await self.raw_peer(peer))
        entering = asyncio.ensure_future(client.__aenter__())
        try:
            first = await asyncio.wait_for(requests.get(), 5)
            self.assertFalse(entering.done())
            entering.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await entering
            await asyncio.wait_for(closed.wait(), 3)
            self.assertIs(await asyncio.wait_for(client.__aenter__(), 5), client)
            self.assertEqual(first, await asyncio.wait_for(requests.get(), 3))
            await asyncio.wait_for(client.stop(), 5)
        finally:
            release.set()
            entering.cancel()
            await asyncio.gather(entering, return_exceptions=True)
            await asyncio.wait_for(client.stop(), 5)

    async def test_cov_iterator_releases_lease_and_ends_after_stop(self):
        hub, _ = await self.serving_hub()
        client = self.client(await hub.url())
        await asyncio.wait_for(client.__aenter__(), 5)
        iterator = await client.cov_notifications()
        receiving = asyncio.ensure_future(iterator.__anext__())
        await asyncio.wait_for(client.stop(), 5)
        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(receiving, 3)
