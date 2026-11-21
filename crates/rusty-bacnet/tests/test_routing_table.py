"""The read-only routing-table query (#2612), against a real three-port router."""
import asyncio
import copy
import pickle
import socket
import unittest

import rusty_bacnet as rb
from test_bip_modes import accepted_registration
from test_router import port


class RoutingTableTests(unittest.IsolatedAsyncioTestCase):
    @staticmethod
    async def cleanup(method):
        await asyncio.wait_for(method(), 10)

    async def asyncSetUp(self):
        self.router = rb.BACnetRouter(
            [port(n, bbmd=rb.Bbmd(accept_foreign_devices=True, alone_on_subnet=True))
             for n in (1, 5, 7)])
        await self.router.start()
        self.addAsyncCleanup(self.cleanup, self.router.close)
        self.ports = await self.router.ports()
        # The client is a foreign device on network 1, so the router is a
        # station on its own link.
        self.client = rb.BACnetClient(
            interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
            foreign_device=rb.ForeignDevice(self.ports[0].address, ttl=60))
        await self.client.__aenter__()
        self.addAsyncCleanup(self.cleanup, self.client.stop)
        await accepted_registration(self.client)

    async def test_the_table_of_a_three_port_router_is_read_twice_unchanged(self):
        first = await self.client.routing_table(self.ports[0].mac)
        again = await self.client.routing_table(self.ports[0].mac)
        self.assertIsInstance(first, rb.RoutingTable)
        self.assertFalse(first.truncated)
        self.assertGreaterEqual(first.acks, 1)
        networks = sorted(entry.network for entry in first.entries)
        self.assertEqual(networks, [1, 5, 7])
        self.assertEqual(len({entry.port_id for entry in first.entries}), 3)
        for entry in first.entries:
            self.assertIsInstance(entry, rb.RoutingTableEntry)
            self.assertEqual(entry.port_info, b"")
        # A query changes nothing: the same table again, and the router
        # still announces the same routes.
        self.assertEqual(
            [(e.network, e.port_id) for e in first.entries],
            [(e.network, e.port_id) for e in again.entries])
        [router] = await self.client.discover_routers(wait=1.5)
        self.assertEqual(router.networks, [5, 7])

    async def test_seated_client_reads_the_same_table_on_its_endpoint_socket(self):
        async with rb.BipEndpoint(2612, interface="127.0.0.1", port=0,
                                  broadcast_address="127.0.0.1") as endpoint:
            address = await endpoint.local_address()
            frames = await endpoint.frames()
            client = await endpoint.client()
            table = await client.routing_table(self.ports[0].mac)
            reference = await self.client.routing_table(self.ports[0].mac)
            self.assertEqual([(e.network, e.port_id) for e in table.entries],
                             [(e.network, e.port_id) for e in reference.entries])
            async with asyncio.timeout(5):
                async for frame in frames:
                    if frame.direction == "outbound" and frame.octets[4:8] == bytes([1, 0x84, 6, 0]):
                        self.assertEqual(frame.local_address, address)
                        break
                else:
                    self.fail("the endpoint's socket did not send the routing-table query")
            await frames.aclose()

    async def test_the_router_may_be_given_as_text(self):
        table = await self.client.routing_table(self.ports[0].address)
        self.assertEqual(sorted(e.network for e in table.entries), [1, 5, 7])

    async def test_records_are_frozen_and_refuse_copy_and_pickle(self):
        table = await self.client.routing_table(self.ports[0].mac)
        entry = table.entries[0]
        for record in (table, entry):
            with self.assertRaises(AttributeError):
                record.acks = 3
            with self.assertRaises(TypeError):
                copy.copy(record)
            with self.assertRaises(TypeError):
                pickle.dumps(record)

    async def test_silence_is_a_timeout_not_an_empty_table(self):
        # Nothing listens at this loopback address and port.
        nobody = bytes([127, 0, 0, 1, 0x1F, 0x90])
        with self.assertRaises(rb.BacnetTimeoutError):
            await self.client.routing_table(nobody, wait=0.3)

    async def test_bounds_and_arguments_are_checked(self):
        mac = self.ports[0].mac
        for bad in ({"wait": -1.0}, {"quiet": 3601.0}, {"total": float("nan")},
                    {"max_entries": 0}, {"wait": 11.0}):
            with self.assertRaises(ValueError, msg=bad):
                await self.client.routing_table(mac, **bad)
        with self.assertRaises(TypeError):
            await self.client.routing_table(7)
        # Keyword-only bounds.
        with self.assertRaises(TypeError):
            await self.client.routing_table(mac, 1.0)

    async def test_a_reject_from_the_router_raises_with_reason_and_network(self):
        # A scripted "router" on loopback: it answers the query with a
        # Reject-Message-To-Network, first for a DNET (reason 2, ignored),
        # then unknown message type (reason 3, which ends the call).
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        sock.setblocking(False)
        self.addCleanup(sock.close)
        port_number = sock.getsockname()[1]
        mac = bytes([127, 0, 0, 1]) + port_number.to_bytes(2, "big")
        client = rb.BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")
        await client.__aenter__()
        self.addAsyncCleanup(self.cleanup, client.stop)

        async def answer():
            loop = asyncio.get_running_loop()
            data, peer = await loop.sock_recvfrom(sock, 2048)
            # BVLL Original-Unicast-NPDU, then NPDU version 1, network
            # message, type 3, a reason and a network number.
            self.assertEqual(data[4:8], b"\x01\x84\x06\x00")
            for reason, network in ((2, 77), (3, 0)):
                npdu = bytes([1, 0x80, 3, reason]) + network.to_bytes(2, "big")
                await loop.sock_sendto(sock, b"\x81\x0a" + (4 + len(npdu)).to_bytes(2, "big") + npdu, peer)

        answering = asyncio.ensure_future(answer())
        with self.assertRaises(rb.BacnetNetworkRejectError) as caught:
            await client.routing_table(mac, wait=2.0)
        await answering
        self.assertEqual(caught.exception.reason, 3)
        self.assertEqual(caught.exception.network, 0)
        self.assertFalse(isinstance(caught.exception, rb.BacnetTimeoutError))

    async def test_the_entry_bound_marks_a_cut_table(self):
        table = await self.client.routing_table(self.ports[0].mac, max_entries=2)
        self.assertEqual(len(table.entries), 2)
        self.assertTrue(table.truncated)

    async def test_the_exception_classes(self):
        self.assertTrue(issubclass(rb.BacnetNetworkRejectError, rb.BacnetError))
        self.assertFalse(issubclass(rb.BacnetNetworkRejectError, rb.BacnetTimeoutError))
        self.assertIsNone(rb.BacnetNetworkRejectError.reason)
        self.assertIsNone(rb.BacnetNetworkRejectError.network)


if __name__ == "__main__":
    unittest.main()
