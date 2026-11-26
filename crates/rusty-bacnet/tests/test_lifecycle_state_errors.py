"""The bindings' own lifecycle checks raise BacnetStateError (#2819).

A client, server, endpoint, router or hub called before its start, after
its stop, or started twice raises BacnetStateError, as the same condition
noticed by the Rust stack does: still a RuntimeError, with the text these
checks always raised, and a ``state`` saying which state refused the call.
Every check here runs on 127.0.0.1 and sends nothing to a device.
"""
import asyncio
import unittest

import rusty_bacnet as rb

LOOPBACK = {"interface": "127.0.0.1", "port": 0, "broadcast_address": "127.0.0.1"}
AI1 = rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 1)
PV = rb.PropertyIdentifier.PRESENT_VALUE
CLIENT_NOT_STARTED = "client not started — use 'async with'"
ENDPOINT_NOT_STARTED = "endpoint not started — use 'async with' or await start()"
ROUTER_NOT_STARTED = "router not started or already closing"


async def finish(method):
    await asyncio.wait_for(method(), 10)


class StateErrorCase(unittest.IsolatedAsyncioTestCase):
    def check(self, error, state, text):
        self.assertIsInstance(error, rb.BacnetStateError)
        self.assertEqual(error.state, state)
        self.assertEqual(str(error), text)

    async def refused(self, call, state, text):
        """`call()`, made twice, raises BacnetStateError with `state` and
        `text`, which ``except RuntimeError`` and ``except BacnetError``
        each catch."""
        for catch in (RuntimeError, rb.BacnetError):
            with self.subTest(text=text, caught_as=catch.__name__):
                try:
                    await call()
                except catch as error:
                    self.check(error, state, text)
                else:
                    self.fail(f"{text!r} was not raised")

    def refused_now(self, call, state, text):
        """`refused` for a call that raises before it returns an awaitable."""
        for catch in (RuntimeError, rb.BacnetError):
            with self.subTest(text=text, caught_as=catch.__name__):
                try:
                    call()
                except catch as error:
                    self.check(error, state, text)
                else:
                    self.fail(f"{text!r} was not raised")


class ClientStateTests(StateErrorCase):
    async def test_before_start_twice_and_after_stop(self):
        client = rb.BACnetClient(**LOOPBACK)
        calls = (
            lambda: client.read_property("127.0.0.1:47808", AI1, PV),
            # The high-level client checks the same way.
            lambda: client.device("127.0.0.1:47808"),
        )
        for call in calls:
            await self.refused(call, "not_running", CLIENT_NOT_STARTED)
        await asyncio.wait_for(client.__aenter__(), 10)
        try:
            await self.refused(client.__aenter__, "already_started", "client already started")
        finally:
            await finish(client.stop)
        # A standalone client starts again after its stop.
        for call in calls:
            await self.refused(call, "not_running", CLIENT_NOT_STARTED)


class ServerStateTests(StateErrorCase):
    async def test_before_start_twice_and_after_stop(self):
        server = rb.BACnetServer(281901, "State Server", **LOOPBACK)
        await self.refused(server.local_address, "not_running", "server not started")
        await asyncio.wait_for(server.start(), 10)
        try:
            await self.refused(server.start, "already_started",
                               "cannot start() — server is already running")
            self.refused_now(lambda: server.add_analog_input(2, "AI 2"), "already_started",
                             "cannot add objects after start() — server is already running")
        finally:
            await finish(server.stop)
        await self.refused(server.local_address, "not_running", "server not started")


class HubStateTests(StateErrorCase):
    async def test_before_start(self):
        # No file is read and nothing binds before start().
        hub = rb.ScHub(listen="127.0.0.1:0", cert="absent-cert.pem", key="absent-key.pem",
                       vmac=b"\x02\0\0\0\0\x01", ca_cert="absent-ca.pem",
                       device_uuid=bytes.fromhex("9a21f1641a15454d9ed7e3a2710d7002"))
        await self.refused(hub.status, "not_running", "hub not started")
        await self.refused(hub.shutdown_gracefully, "not_running", "hub not started")


def router_port():
    return rb.BipRouterPort(1, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                            bbmd=rb.Bbmd(accept_foreign_devices=True, alone_on_subnet=True))


class RouterStateTests(StateErrorCase):
    async def test_before_start_twice_and_after_close(self):
        router = rb.BACnetRouter([router_port()], application="client")
        for call in (router.ports, router.counters, router.routes):
            await self.refused(call, "not_running", ROUTER_NOT_STARTED)
        # A stack on the router's application entity checks it too.
        share = rb.BACnetClient(transport="router", router=router)
        await self.refused(
            share.__aenter__, "not_running",
            ROUTER_NOT_STARTED + ": start the router before the client on its application entity")
        await asyncio.wait_for(router.start(), 10)
        try:
            await self.refused(router.start, "already_started", "router already started")
            self.assertEqual(len(await router.ports()), 1)
        finally:
            await finish(router.close)
        # A router closed cleanly starts again.
        await self.refused(router.ports, "not_running", ROUTER_NOT_STARTED)


class EndpointStateTests(StateErrorCase):
    async def test_before_start_while_running_and_after_close(self):
        endpoint = rb.BipEndpoint(281902, device_name="State Endpoint", **LOOPBACK)
        for call, text in (
            (endpoint.status, "endpoint not started"),
            (endpoint.local_address, "endpoint is not active"),
            (endpoint.foreign_registration, "endpoint not started"),
            (endpoint.frames, "endpoint not started"),
            (endpoint.broadcast_i_am, ENDPOINT_NOT_STARTED),
            (endpoint.client, ENDPOINT_NOT_STARTED),
            (endpoint.server, ENDPOINT_NOT_STARTED),
            (lambda: endpoint.read_bdt("127.0.0.1:47808"), "endpoint not started"),
        ):
            await self.refused(call, "not_running", text)
        await asyncio.wait_for(endpoint.start(), 10)
        try:
            self.refused_now(
                lambda: endpoint.add_analog_input(5, "AI 5"), "already_started",
                "cannot add objects while endpoint is starting, running, or stopping")
            client = await endpoint.client()
            server = await endpoint.server()
            self.assertIsInstance(await server.local_address(), str)
        finally:
            await finish(endpoint.close)
        # The endpoint starts again; the client and server of the run that
        # ended never do.
        await self.refused(endpoint.status, "not_running", "endpoint not started")
        await self.refused(lambda: endpoint.read_fdt("127.0.0.1:47808"), "not_running",
                           "endpoint not started")
        await self.refused(
            lambda: client.read_property("127.0.0.1:47808", AI1, PV), "closed",
            "this client ran on an endpoint and is stopped, or its endpoint is: ask the "
            "running endpoint for another with 'await endpoint.client()'")
        await self.refused(
            server.local_address, "closed",
            "the endpoint this server came from is closed, and its server stopped with it; "
            "after a restart, await endpoint.server() for the new one")


if __name__ == "__main__":
    unittest.main()
