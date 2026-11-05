"""ScEndpoint hub-connector events, link status, reconnect and failover (#1946)."""
import ast
import asyncio
import inspect
import pickle
import unittest

from rusty_bacnet import (
    ObjectIdentifier, ObjectType, PropertyIdentifier, ScEndpoint, ScEvent, ScLinkStatus,
    ScReconnect,
)
from sc_client_fixture import ClientScFixture, mtls
from test_endpoint_stub_parity import (
    installed_stub, runtime_arg_kinds, stub_arg_kinds, stub_classes, stub_method,
)

FAILOVER_VMAC = b"\x02\0\0\0\0\x0b"
FAILOVER_UUID = bytes.fromhex("5b0d6a1e3c7f4e2a9d81c4f0e6b2a713")
ENDPOINT_VMAC = b"\x02\0\0\0\0\x05"
ENDPOINT_INSTANCE = 4105
ON_PRIMARY = ("connected", "primary", mtls.HUB_VMAC)


def link(status):
    return (status.state, status.active_hub, status.hub_vmac)


class ScEndpointOptionTests(unittest.TestCase):
    @staticmethod
    def kwargs(**overrides):
        # Construction reads no file and dials nothing, so these paths pass.
        settings = dict(device_instance=1, sc_hub="wss://localhost:1", sc_vmac=ENDPOINT_VMAC,
                        sc_ca_cert="/no/ca", sc_client_cert="/no/cert",
                        sc_client_key="/no/key", sc_device_uuid=b"a" * 16)
        settings.update(overrides)
        return settings

    def test_options_are_named_and_checked_as_on_the_client(self):
        for keyword in ("reconnect", "sc_failover_hub", "sc_connect_timeout_ms"):
            parameter = inspect.signature(ScEndpoint).parameters[keyword]
            self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
            self.assertIsNone(parameter.default)
        ScEndpoint(**self.kwargs(sc_failover_hub="wss://localhost:2",
                                 reconnect=ScReconnect(), sc_connect_timeout_ms=500))
        with self.assertRaisesRegex(ValueError, "requires reconnect"):
            ScEndpoint(**self.kwargs(sc_failover_hub="wss://localhost:2"))
        with self.assertRaisesRegex(ValueError, "at least 1"):
            ScEndpoint(**self.kwargs(sc_connect_timeout_ms=0))
        with self.assertRaises(TypeError):
            ScEndpoint(**self.kwargs(reconnect={}))

    def test_stub_matches_runtime(self):
        endpoint = stub_classes(installed_stub())["ScEndpoint"]
        for name, returns in (("connection_events", "Awaitable[ScEventIterator]"),
                              ("connection_status", "Awaitable[ScLinkStatus]")):
            with self.subTest(method=name):
                node = stub_method(endpoint, name)
                self.assertEqual(ast.unparse(node.returns), returns)
                self.assertEqual(runtime_arg_kinds(getattr(ScEndpoint, name)),
                                 stub_arg_kinds(node))


class ScEndpointLinkTests(ClientScFixture):
    def endpoint(self, url, **overrides):
        settings = dict(device_instance=ENDPOINT_INSTANCE, sc_hub=url, sc_vmac=ENDPOINT_VMAC,
                        sc_ca_cert=self.path("site.pem"),
                        sc_client_cert=self.path("server.pem"),
                        sc_client_key=self.path("server.key"),
                        sc_device_uuid=mtls.SERVER_UUID)
        settings.update(overrides)
        endpoint = ScEndpoint(**settings)
        self.addAsyncCleanup(self.close_endpoint, endpoint)
        return endpoint

    async def close_endpoint(self, endpoint):
        await asyncio.wait_for(endpoint.close(), 5)

    async def started_hub(self, **overrides):
        hub = self.hub(**overrides)
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 5)
        return hub

    async def until(self, events, kind):
        """Read events up to and including the next of `kind`, with the
        loop time each arrived."""
        seen, times = [], []

        async def collect():
            async for event in events:
                self.assertIsInstance(event, ScEvent)
                seen.append(event)
                times.append(asyncio.get_running_loop().time())
                if event.kind == kind:
                    return
        await asyncio.wait_for(collect(), 30)
        return seen, times

    async def read_endpoint(self, client):
        """The endpoint's server role answers the client through the hub."""
        name = await asyncio.wait_for(client.read_property(
            ":".join(f"{b:02x}" for b in ENDPOINT_VMAC),
            ObjectIdentifier(ObjectType.DEVICE, ENDPOINT_INSTANCE),
            PropertyIdentifier.OBJECT_NAME), 10)
        self.assertEqual(name.value, "BACnet Device")

    async def test_failover_and_restore_match_the_client(self):
        primary = await self.started_hub()
        failover = await self.started_hub(vmac=FAILOVER_VMAC, device_uuid=FAILOVER_UUID)
        relay = await self.relay(await primary.url())
        # 3 s: a TLS handshake on a loaded Windows runner can pass 1 s, and
        # the first connect must reach the primary, not fall over.
        options = dict(sc_failover_hub=await failover.url(), sc_connect_timeout_ms=3_000,
                       reconnect=ScReconnect(initial_delay_ms=50, max_delay_ms=50,
                                             max_retries=1))
        endpoint = self.endpoint(relay.url, **options)
        client = self.client(relay.url, apdu_timeout_ms=3_000, **options)
        for method in (endpoint.connection_events, endpoint.connection_status):
            with self.assertRaisesRegex(RuntimeError, "not started"):
                await method()
        await asyncio.wait_for(endpoint.start(), 10)
        await asyncio.wait_for(client.__aenter__(), 10)

        # The first subscriber starts at the first connect (#1908); a later
        # one is a late subscriber, with nothing yet.
        events = await endpoint.connection_events()
        late = await endpoint.connection_events()
        first = await asyncio.wait_for(events.__anext__(), 10)
        self.assertEqual((first.kind, first.hub, first.hub_vmac), ON_PRIMARY)
        with self.assertRaises(asyncio.TimeoutError):
            await asyncio.wait_for(late.__anext__(), 0.2)
        client_events = await client.connection_events()
        self.assertEqual((await asyncio.wait_for(client_events.__anext__(), 10)).kind,
                         "connected")
        status = await endpoint.connection_status()
        self.assertIsInstance(status, ScLinkStatus)
        self.assertEqual(link(status), ON_PRIMARY)
        self.assertEqual(link(await client.connection_status()), ON_PRIMARY)
        with self.assertRaises(TypeError):
            pickle.dumps(status)
        await self.read_endpoint(client)

        # Hold new connections to the primary and drop the live ones: each
        # redial stalls until the connect timeout, then the failover hub
        # takes over.
        relay.pause()
        await relay.disconnect()
        on_failover = ("connected", "failover", FAILOVER_VMAC)
        for iterator in (events, client_events):
            seen, _ = await self.until(iterator, "connected")
            kinds = [event.kind for event in seen]
            self.assertEqual((kinds[0], seen[0].hub), ("disconnected", "primary"), seen)
            self.assertEqual(kinds[-2:], ["failover_engaged", "connected"], seen)
            self.assertEqual((seen[-1].hub, seen[-1].hub_vmac), on_failover[1:])
            failed = [event for event in seen if event.kind == "connect_failed"]
            self.assertTrue(failed and all(event.cause == "dial_timeout" for event in failed),
                            seen)
        self.assertEqual(link(await endpoint.connection_status()), on_failover)
        self.assertEqual(link(await client.connection_status()), on_failover)
        # The server role carries on through the failover hub.
        await self.read_endpoint(client)

        relay.resume()
        for iterator in (events, client_events):
            seen, _ = await self.until(iterator, "connected")
            self.assertEqual([event.kind for event in seen][-2:],
                             ["primary_restored", "connected"], seen)
            self.assertEqual(seen[-1].hub, "primary")
        self.assertEqual(link(await endpoint.connection_status()), ON_PRIMARY)
        self.assertEqual(link(await client.connection_status()), ON_PRIMARY)
        await self.read_endpoint(client)
        self.assertEqual(events.missed, 0)

        # Closing ends the iterators and refuses new calls.
        await asyncio.wait_for(endpoint.close(), 10)
        with self.assertRaises(StopAsyncIteration):
            while True:
                await asyncio.wait_for(events.__anext__(), 10)
        for method in (endpoint.connection_events, endpoint.connection_status):
            with self.assertRaisesRegex(RuntimeError, "not started"):
                await method()

    async def test_reconnect_redials_the_hub_within_the_connect_timeout(self):
        hub = await self.started_hub()
        relay = await self.relay(await hub.url())
        # 2 s: well under the 10 s default this test tells apart, with room
        # for a Windows runner's TLS handshake on the first connect.
        endpoint = self.endpoint(relay.url, sc_connect_timeout_ms=2_000, reconnect=ScReconnect(
            initial_delay_ms=50, max_delay_ms=50, max_retries=50))
        await asyncio.wait_for(endpoint.start(), 10)
        events = await endpoint.connection_events()
        first = await asyncio.wait_for(events.__anext__(), 10)
        self.assertEqual(first.kind, "connected")

        relay.pause()
        await relay.disconnect()
        seen, times = await self.until(events, "connect_failed")
        kinds = [event.kind for event in seen]
        self.assertEqual(kinds, ["disconnected", "reconnect_attempt", "connect_failed"], seen)
        self.assertEqual((seen[-1].cause, seen[-1].dial_stage), ("dial_timeout", "tls"))
        # The 2 s bound, not the 10 s default.
        self.assertLess(times[-1] - times[1], 5)
        status = await endpoint.connection_status()
        self.assertEqual(status.active_hub, "primary")
        self.assertIsNone(status.hub_vmac)

        relay.resume()
        seen, _ = await self.until(events, "connected")
        self.assertEqual((seen[-1].hub, seen[-1].hub_vmac), ("primary", mtls.HUB_VMAC))
        self.assertEqual(link(await endpoint.connection_status()), ON_PRIMARY)

    async def test_without_the_options_a_lost_hub_stays_lost(self):
        hub = await self.started_hub()
        relay = await self.relay(await hub.url())
        endpoint = self.endpoint(relay.url)
        await asyncio.wait_for(endpoint.start(), 10)
        events = await endpoint.connection_events()
        first = await asyncio.wait_for(events.__anext__(), 10)
        self.assertEqual((first.kind, first.hub, first.hub_vmac), ON_PRIMARY)

        await relay.disconnect()
        lost = await asyncio.wait_for(events.__anext__(), 10)
        self.assertEqual((lost.kind, lost.hub), ("disconnected", "primary"))
        # No reconnect attempt follows.
        with self.assertRaises(asyncio.TimeoutError):
            await asyncio.wait_for(events.__anext__(), 1)
        status = await endpoint.connection_status()
        self.assertEqual((status.state, status.hub_vmac), ("disconnected", None))

        # A restart subscribes anew: its first caller sees its own connect.
        await asyncio.wait_for(endpoint.close(), 10)
        await asyncio.wait_for(endpoint.start(), 10)
        again = await endpoint.connection_events()
        first = await asyncio.wait_for(again.__anext__(), 10)
        self.assertEqual((first.kind, first.hub), ("connected", "primary"))


if __name__ == "__main__":
    unittest.main()
