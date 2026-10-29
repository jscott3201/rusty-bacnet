"""A client, and a server, on a router's own application entity (#2071).

Every network is a B/IP port on 127.0.0.1 with its own UDP port, so each
device reaches its network as a foreign device of the router's BBMD port,
as in test_router.py.
"""
import asyncio
import errno
import unittest

import rusty_bacnet as rb
from test_bip_modes import accepted_registration, eventually

PV = rb.PropertyIdentifier.PRESENT_VALUE
AI0 = rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 0)
BASE = 207100


def bbmd_port(network, **kw):
    return rb.BipRouterPort(network, interface="127.0.0.1", port=0,
                            broadcast_address="127.0.0.1",
                            bbmd=rb.Bbmd(accept_foreign_devices=True, alone_on_subnet=True),
                            **kw)


async def cleanup(method):
    await asyncio.wait_for(method(), 10)


class RouterApplicationSurfaceTests(unittest.TestCase):
    def test_keywords_are_checked_before_any_io(self):
        ports = [bbmd_port(1)]
        self.assertIsNone(rb.BACnetRouter(ports).application)
        client_router = rb.BACnetRouter(ports, application="client")
        split = rb.BACnetRouter(ports, application="client_and_server")
        self.assertEqual(client_router.application, "client")
        self.assertEqual(split.application, "client_and_server")
        with self.assertRaisesRegex(ValueError, "application must be"):
            rb.BACnetRouter(ports, application="server")
        with self.assertRaises(TypeError):
            rb.BACnetRouter(ports, "client")  # keyword only
        with self.assertRaisesRegex(ValueError, "needs router="):
            rb.BACnetClient(transport="router")
        with self.assertRaisesRegex(ValueError, "only to transport='router'"):
            rb.BACnetClient(router=client_router)
        with self.assertRaisesRegex(ValueError, "only to transport='router'"):
            rb.BACnetServer(1, router=split)
        with self.assertRaisesRegex(ValueError, "this router has no application"):
            rb.BACnetClient(transport="router", router=rb.BACnetRouter(ports))
        with self.assertRaisesRegex(ValueError, "this router has 'client'"):
            rb.BACnetServer(1, transport="router", router=client_router)
        # A split entity forgets a request 60 seconds after its last activity.
        with self.assertRaisesRegex(ValueError, "apdu_timeout_ms must be under 60000"):
            rb.BACnetClient(transport="router", router=split, apdu_timeout_ms=60000)
        with self.assertRaisesRegex(ValueError, "apdu_segment_timeout_ms must be under 60000"):
            rb.BACnetServer(1, transport="router", router=split, apdu_segment_timeout_ms=60000)
        rb.BACnetClient(transport="router", router=client_router, apdu_timeout_ms=60000)
        rb.BACnetClient(transport="router", router=split, apdu_timeout_ms=59999)
        rb.BACnetServer(1, transport="router", router=split)


class RouterApplicationTests(unittest.IsolatedAsyncioTestCase):
    async def device(self, instance, bbmd_address, value):
        """A server with one analog input, a foreign device of `bbmd_address`."""
        server = rb.BACnetServer(instance, interface="127.0.0.1", port=0,
                                 foreign_device=rb.ForeignDevice(bbmd_address, ttl=60))
        server.add_analog_input(0, f"Sensor {instance}", 62, value)
        await server.start()
        self.addAsyncCleanup(cleanup, server.stop)
        await accepted_registration(server)
        return server

    async def router_client(self, router, **kw):
        client = rb.BACnetClient(transport="router", router=router, apdu_timeout_ms=2000, **kw)
        await client.__aenter__()
        self.addAsyncCleanup(cleanup, client.stop)
        return client

    async def find(self, client, instance):
        """Discover `instance` and return its network."""
        async def found():
            devices = await client.discover(timeout_ms=500, low_limit=instance,
                                            high_limit=instance)
            return [d for d in devices if d.object_identifier.instance == instance]
        [device] = await eventually(found, seconds=10)
        return device.source_network

    async def test_the_client_reads_a_device_on_each_attached_network(self):
        router = rb.BACnetRouter([bbmd_port(1), bbmd_port(5)], application="client")
        await router.start()
        self.addAsyncCleanup(cleanup, router.close)
        ports = await router.ports()
        for network, port in zip((1, 5), ports):
            await self.device(BASE + network, port.address, 20.5 + network)
        client = await self.router_client(router)
        for network in (1, 5):
            # Every APDU reaches the client with its network as the source,
            # so each device is a routed device, whether attached or not.
            self.assertEqual(await self.find(client, BASE + network), network)
            value = await asyncio.wait_for(
                client.read_property_from_device(BASE + network, AI0, PV), 10)
            self.assertAlmostEqual(value.value, 20.5 + network)
        # An ip:port names no network, and the router's application has none.
        with self.assertRaisesRegex(rb.BacnetError, "no network of its own"):
            await asyncio.wait_for(client.read_property(ports[0].address, AI0, PV), 10)

    async def test_the_client_reads_a_device_behind_a_second_router(self):
        near = rb.BACnetRouter([bbmd_port(1), bbmd_port(5)], application="client")
        await near.start()
        self.addAsyncCleanup(cleanup, near.close)
        near_ports = await near.ports()
        # The far router's network 5 port is a foreign device of the near
        # router's, so the two exchange network 5 broadcasts.
        far = rb.BACnetRouter([
            rb.BipRouterPort(5, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                             foreign_device=rb.ForeignDevice(near_ports[1].address, ttl=60)),
            bbmd_port(7),
        ])
        await far.start()
        self.addAsyncCleanup(cleanup, far.close)
        far_ports = await far.ports()
        await self.device(BASE + 7, far_ports[1].address, 27.5)
        client = await self.router_client(near)
        self.assertEqual(await self.find(client, BASE + 7), 7)

        async def read():
            try:
                return await client.read_property_from_device(BASE + 7, AI0, PV)
            except rb.BacnetTransportError as error:
                # No route to 7 yet: the router searches for it.
                self.assertEqual(error.errno, errno.ENETUNREACH)
                return None
        value = await eventually(read, seconds=15)
        self.assertAlmostEqual(value.value, 27.5)
        routes = {r.network_number: r for r in await near.routes()}
        self.assertFalse(routes[7].is_direct)
        self.assertEqual(routes[7].next_hop, far_ports[0].mac)

    async def test_a_server_and_a_client_share_the_entity(self):
        router = rb.BACnetRouter([bbmd_port(1), bbmd_port(5)],
                                 application="client_and_server")
        await router.start()
        self.addAsyncCleanup(cleanup, router.close)
        ports = await router.ports()
        hosted = rb.BACnetServer(BASE + 100, transport="router", router=router)
        hosted.add_analog_input(0, "Hosted sensor", 62, 42.25)
        await hosted.start()
        self.addAsyncCleanup(cleanup, hosted.stop)
        await self.device(BASE + 5, ports[1].address, 25.5)
        client = await self.router_client(router)
        remote = rb.BACnetClient(interface="127.0.0.1", port=0,
                                 foreign_device=rb.ForeignDevice(ports[0].address, ttl=60))
        await remote.__aenter__()
        self.addAsyncCleanup(cleanup, remote.stop)
        await accepted_registration(remote)
        self.assertEqual(await self.find(client, BASE + 5), 5)
        # Both at once: the remote client reads the router's server at the
        # router's address on network 1, while the router's client reads
        # the device on network 5.
        hosted_value, remote_value = await asyncio.wait_for(asyncio.gather(
            remote.read_property(ports[0].address, AI0, PV),
            client.read_property_from_device(BASE + 5, AI0, PV),
        ), 10)
        self.assertAlmostEqual(hosted_value.value, 42.25)
        self.assertAlmostEqual(remote_value.value, 25.5)
        # The hosted server answers Who-Is too, at the router's address.
        [found] = await eventually(lambda: remote.discover(
            timeout_ms=1000, low_limit=BASE + 100, high_limit=BASE + 100), seconds=10)
        self.assertEqual(found.mac_address, ports[0].mac)
        self.assertIsNone(found.source_network)

    async def test_one_stack_per_role_per_router_start(self):
        router = rb.BACnetRouter([bbmd_port(1)], application="client")
        client = rb.BACnetClient(transport="router", router=router)
        with self.assertRaisesRegex(RuntimeError, "router not started"):
            await client.__aenter__()
        await router.start()
        self.addAsyncCleanup(cleanup, router.close)
        await client.__aenter__()
        second = rb.BACnetClient(transport="router", router=router)
        with self.assertRaisesRegex(RuntimeError, "already taken"):
            await second.__aenter__()
        await client.stop()
        # The port ended with the client; the next router start makes another.
        with self.assertRaisesRegex(RuntimeError, "already taken"):
            await client.__aenter__()
        await router.close()
        await router.start()
        await client.__aenter__()
        await client.stop()

    async def test_a_refused_server_start_leaves_the_port_for_the_next(self):
        router = rb.BACnetRouter([bbmd_port(1)], application="client_and_server")
        await router.start()
        self.addAsyncCleanup(cleanup, router.close)
        broken = rb.BACnetServer(BASE + 200, transport="router", router=router)
        broken.add_analog_input(0, "Same name", 62, 1.0)
        broken.add_analog_input(1, "Same name", 62, 2.0)
        with self.assertRaisesRegex(ValueError, "duplicate object name"):
            await broken.start()
        hosted = rb.BACnetServer(BASE + 201, transport="router", router=router)
        hosted.add_analog_input(0, "Hosted sensor", 62, 3.0)
        await hosted.start()
        self.addAsyncCleanup(cleanup, hosted.stop)
        # The port went to the server that started, and only once.
        late = rb.BACnetServer(BASE + 202, transport="router", router=router)
        with self.assertRaisesRegex(RuntimeError, "already taken"):
            await late.start()

    async def test_closing_the_router_fails_the_client_cleanly(self):
        router = rb.BACnetRouter([bbmd_port(1)], application="client")
        await router.start()
        self.addAsyncCleanup(cleanup, router.close)
        [port] = await router.ports()
        await self.device(BASE + 1, port.address, 21.5)
        client = await self.router_client(router)
        await self.find(client, BASE + 1)
        await client.read_property_from_device(BASE + 1, AI0, PV)
        await asyncio.wait_for(router.close(), 10)
        # A request after close fails at once, not by timing out.
        with self.assertRaises(rb.BacnetTransportError) as caught:
            await asyncio.wait_for(client.read_property_from_device(BASE + 1, AI0, PV), 1)
        self.assertEqual(caught.exception.errno, errno.ENOTCONN)
        with self.assertRaises(rb.BacnetTransportError):
            await asyncio.wait_for(client.who_is(), 1)
        await asyncio.wait_for(client.stop(), 10)

    async def test_a_dropped_router_leaves_the_client_failing_cleanly(self):
        router = rb.BACnetRouter([bbmd_port(1)], application="client")
        await router.start()
        client = await self.router_client(router)
        del router
        async def stopped():
            try:
                await asyncio.wait_for(client.who_is(), 1)
            except rb.BacnetTransportError as error:
                return error.errno == errno.ENOTCONN
            return False
        self.assertTrue(await eventually(stopped, seconds=10))


if __name__ == "__main__":
    unittest.main()
