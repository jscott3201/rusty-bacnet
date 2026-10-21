"""SC hub-connector events, link status and the failover hub (#1829)."""
import ast
import asyncio
import inspect
import pickle
import time
import unittest

import rusty_bacnet
from rusty_bacnet import BACnetClient, BACnetServer, ScEvent, ScLinkStatus, ScReconnect
from sc_client_fixture import ClientScFixture, mtls
from test_endpoint_stub_parity import (
    installed_stub, runtime_arg_kinds, stub_arg_kinds, stub_classes, stub_method,
)

FAILOVER_VMAC = b"\x02\0\0\0\0\x0b"
FAILOVER_UUID = bytes.fromhex("5b0d6a1e3c7f4e2a9d81c4f0e6b2a713")


class ScHubOptionTests(unittest.TestCase):
    def test_options_are_sc_only_and_failover_needs_reconnect(self):
        sc = dict(transport="sc", sc_ca_cert="/no/ca", sc_client_cert="/no/cert",
                  sc_client_key="/no/key", sc_device_uuid=b"a" * 16)
        for keyword in ("sc_failover_hub", "sc_connect_timeout_ms"):
            parameter = inspect.signature(BACnetClient).parameters[keyword]
            self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
            self.assertIsNone(parameter.default)
        # Construction reads no file and dials nothing, so these paths pass.
        BACnetClient(**sc, sc_failover_hub="wss://localhost:1",
                     reconnect=ScReconnect(), sc_connect_timeout_ms=500)
        with self.assertRaisesRegex(ValueError, "requires reconnect"):
            BACnetClient(**sc, sc_failover_hub="wss://localhost:1")
        with self.assertRaisesRegex(ValueError, "at least 1"):
            BACnetClient(**sc, sc_connect_timeout_ms=0)
        for transport in ("bip", "ipv6", "mstp"):
            with self.assertRaisesRegex(ValueError, "only to transport='sc'"):
                BACnetClient(transport=transport, sc_connect_timeout_ms=500)
            with self.assertRaisesRegex(ValueError, "only to transport='sc'"):
                BACnetClient(transport=transport, sc_failover_hub="wss://localhost:1")

    def test_stub_matches_runtime(self):
        classes = stub_classes(installed_stub())
        client = classes["BACnetClient"]
        self.assertEqual(runtime_arg_kinds(BACnetClient),
                         stub_arg_kinds(stub_method(client, "__init__")))
        for name, returns in (("connection_events", "Awaitable[ScEventIterator]"),
                              ("connection_status", "Awaitable[ScLinkStatus]")):
            node = stub_method(client, name)
            self.assertEqual(ast.unparse(node.returns), returns)
            self.assertEqual(runtime_arg_kinds(getattr(BACnetClient, name)),
                             stub_arg_kinds(node))
        for cls, names in (
            ("ScEvent", {"kind", "hub", "hub_vmac", "cause", "cause_detail", "dial_stage",
                         "http_status", "error_class", "error_code", "attempt", "backoff"}),
            ("ScLinkStatus", {"state", "active_hub", "hub_vmac", "heartbeat_age",
                              "heartbeat_rtt"}),
            ("ScEventIterator", {"missed"}),
        ):
            with self.subTest(cls=cls):
                stubbed = {node.name for node in classes[cls].body
                           if isinstance(node, ast.FunctionDef) and node.decorator_list}
                self.assertEqual(stubbed, names)
                runtime = getattr(rusty_bacnet, cls)
                for name in names:
                    self.assertIsInstance(inspect.getattr_static(runtime, name),
                                          type(inspect.getattr_static(ScEvent, "kind")))
        method = stub_method(classes["ScEventIterator"], "__anext__")
        self.assertEqual(ast.unparse(method.returns), "Awaitable[ScEvent]")


class ScConnectionEventTests(ClientScFixture):
    async def failover_hub(self):
        hub = self.hub(vmac=FAILOVER_VMAC, device_uuid=FAILOVER_UUID)
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 5)
        server = BACnetServer(
            3000, transport="sc", sc_hub=await hub.url(), sc_vmac=b"\x02\0\0\0\0\4",
            sc_device_uuid=mtls.SERVER_UUID, sc_ca_cert=self.path("site.pem"),
            sc_client_cert=self.path("server.pem"), sc_client_key=self.path("server.key"))
        self.addAsyncCleanup(self.stop_server, server)
        server.add_analog_input(0, "Failover AI", 64, 72.5)
        await asyncio.wait_for(server.start(), 5)
        return hub

    async def test_losing_the_primary_engages_then_connects_the_failover_hub(self):
        primary, primary_server = await self.serving_hub()
        failover = await self.failover_hub()
        client = self.client(
            await primary.url(), sc_failover_hub=await failover.url(),
            sc_connect_timeout_ms=3_000,
            reconnect=ScReconnect(initial_delay_ms=50, max_delay_ms=50, max_retries=1))
        await asyncio.wait_for(client.__aenter__(), 10)
        await self.read_value(client)
        status = await client.connection_status()
        self.assertIsInstance(status, ScLinkStatus)
        self.assertEqual((status.state, status.active_hub, status.hub_vmac),
                         ("connected", "primary", mtls.HUB_VMAC))
        self.assertGreaterEqual(status.heartbeat_age, 0.0)
        with self.assertRaises(AttributeError):
            status.state = "disconnected"
        with self.assertRaises(TypeError):
            pickle.dumps(status)

        # The first subscriber starts at the first connect (#1908).
        events = await client.connection_events()
        first = await asyncio.wait_for(events.__anext__(), 10)
        self.assertEqual((first.kind, first.hub), ("connected", "primary"))
        # Never read until the end: a subscriber that doesn't keep up must
        # not hold up the connector.
        idle = await client.connection_events()

        await self.stop_server(primary_server)
        await self.stop_hub(primary)
        seen = []

        async def until_failover_connected():
            async for event in events:
                self.assertIsInstance(event, ScEvent)
                seen.append(event)
                if event.kind == "connected":
                    return event
        connected = await asyncio.wait_for(until_failover_connected(), 30)
        kinds = [event.kind for event in seen]
        self.assertEqual(kinds[0], "disconnected", seen)
        self.assertEqual(seen[0].hub, "primary")
        self.assertIsNotNone(seen[0].cause)
        self.assertIn("failover_engaged", kinds, seen)
        self.assertEqual(kinds[-2:], ["failover_engaged", "connected"], seen)
        attempts = [event for event in seen if event.kind == "reconnect_attempt"]
        self.assertTrue(attempts and all(event.hub == "primary" for event in attempts), seen)
        self.assertEqual(attempts[0].attempt, 1)
        self.assertGreaterEqual(attempts[0].backoff, 0.0)
        failed = [event for event in seen if event.kind == "connect_failed"]
        self.assertTrue(failed and all(event.hub == "primary" for event in failed), seen)
        self.assertEqual((connected.hub, connected.hub_vmac), ("failover", FAILOVER_VMAC))
        self.assertEqual(events.missed, 0)

        status = await client.connection_status()
        self.assertEqual((status.state, status.active_hub, status.hub_vmac),
                         ("connected", "failover", FAILOVER_VMAC))
        await self.read_value(client)

        # Stopping the client ends both iterators: the idle one first yields
        # what it held, the same events in the same order.
        await asyncio.wait_for(client.stop(), 10)
        with self.assertRaises(StopAsyncIteration):
            while True:
                event = await asyncio.wait_for(events.__anext__(), 10)
                self.assertNotEqual(event.kind, "failover_engaged")

        async def drain():
            return [event.kind async for event in idle]
        held = await asyncio.wait_for(drain(), 10)
        self.assertEqual(held[:len(kinds)], kinds)
        self.assertEqual(idle.missed, 0)
        with self.assertRaises(RuntimeError):
            await client.connection_events()

    async def test_the_first_subscriber_sees_the_first_connected(self):
        """The first connection_events() call gets the events since the
        build, from the first connected (#1908); a later call is a late
        subscriber, with nothing yet."""
        primary, _server = await self.serving_hub()
        client = self.client(await primary.url())
        await asyncio.wait_for(client.__aenter__(), 10)
        events = await client.connection_events()
        late = await client.connection_events()
        first = await asyncio.wait_for(events.__anext__(), 10)
        self.assertEqual((first.kind, first.hub, first.hub_vmac),
                         ("connected", "primary", mtls.HUB_VMAC))
        self.assertEqual(events.missed, 0)
        with self.assertRaises(asyncio.TimeoutError):
            await asyncio.wait_for(late.__anext__(), 0.2)
        # A restart subscribes anew: its first caller sees its own connect.
        await asyncio.wait_for(client.stop(), 10)
        await asyncio.wait_for(client.__aenter__(), 10)
        again = await client.connection_events()
        first = await asyncio.wait_for(again.__anext__(), 10)
        self.assertEqual((first.kind, first.hub), ("connected", "primary"))

    async def test_a_cancelled_read_puts_the_event_back(self):
        """An event whose __anext__ is cancelled while it is delivered, as a
        timed-out wait_for does, comes back first (#1858)."""
        primary, _server = await self.serving_hub()
        client = self.client(await primary.url())
        await asyncio.wait_for(client.__aenter__(), 10)
        events = await client.connection_events()
        pending = events.__anext__()
        # Blocks the loop: the held connected event is taken and its
        # delivery queued, then the read is given up.
        time.sleep(0.3)
        pending.cancel()
        first = await asyncio.wait_for(events.__anext__(), 10)
        self.assertEqual((first.kind, first.hub), ("connected", "primary"))
        self.assertEqual(events.missed, 0)

    async def test_non_sc_and_stopped_clients_refuse(self):
        client = BACnetClient("127.0.0.1", 0)
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await client.connection_status()
        async with client:
            for method in (client.connection_events, client.connection_status):
                with self.assertRaisesRegex(RuntimeError, "requires an SC client"):
                    await method()


if __name__ == "__main__":
    unittest.main()
