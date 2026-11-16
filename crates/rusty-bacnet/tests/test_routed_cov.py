"""SubscribeCOV, SubscribeCOVProperty and SubscribeCOVPropertyMultiple, and
their cancels, to a station behind a router (#2588).

`subscribe_cov` and `unsubscribe_cov` take a `RoutedTarget` as the routed
calls of #2612/#2617 do, with the same routing rules (a named router, the
route the client learns, a group router refused). The device keeps a
subscription under the address the request came from and notifies back
through its router, so each notification names the station's network and
MAC; a cancel from the same client to the same station finds the
subscription. A raw peer plays the router so the requests can be read off
the wire and notifications sent at will; one class goes through a real
`BACnetRouter` to a real server.

The raw peer (bvll_peer) runs in a thread, since Windows' default event
loop has no add_reader.
"""
import asyncio
import contextlib
import struct
import unittest

import rusty_bacnet as rb
from bvll_peer import RawPeer, bvll, mac
from test_bip_modes import accepted_registration
from test_endpoint_routed import finish, i_am, request_of

O = rb.ObjectType
AI_1 = rb.ObjectIdentifier(O.ANALOG_INPUT, 1)
NEAR = 2588
FAR = 2589
STATION = b"\x0a"
SUBSCRIBE_COV = 5
SUBSCRIBE_COV_PROPERTY = 28
SUBSCRIBE_COV_PROPERTY_MULTIPLE = 30
PV = rb.PropertyIdentifier.PRESENT_VALUE
PROCESS = 7


def relay(network, station, apdu):
    """`apdu` relayed from `station` on `network` as a router relays it, or
    sent direct when `network` is None."""
    if network is None:
        return bvll(0x0A, b"\x01\x00" + apdu)
    source = network.to_bytes(2, "big") + bytes([len(station)]) + station
    return bvll(0x0A, bytes([0x01, 0x08]) + source + apdu)


def notification(process, value):
    """An Unconfirmed-COV-Notification from device FAR about AI 1: a REAL
    Present_Value of `value`, 60 s left."""
    return (b"\x10\x02\x09" + bytes([process])
            + b"\x1c" + ((8 << 22) | FAR).to_bytes(4, "big")
            + b"\x2c\x00\x00\x00\x01\x39\x3c"
            + b"\x4e\x09\x55\x2e\x44" + struct.pack(">f", value) + b"\x2f\x4f")


def cancel_body(process):
    """A SubscribeCOV cancel: the process and the object, nothing else."""
    return b"\x09" + bytes([process]) + b"\x1c\x00\x00\x00\x01"


async def collect(client):
    """A queue each COV notification the client hears goes to, and the
    task that fills it."""
    queue = asyncio.Queue()
    iterator = await client.cov_notifications()

    async def fill():
        async for received in iterator:
            queue.put_nowait(received)

    return queue, asyncio.create_task(fill())


async def stop(task):
    task.cancel()
    with contextlib.suppress(asyncio.CancelledError):
        await task


class CovWire:
    """The raw router's side: keeps each request and where it came from,
    and answers each confirmed request with a SimpleACK from the station."""

    def __init__(self):
        self.requests = []
        self.client = None

    def handle(self, datagram, source):
        if datagram[:2] != b"\x81\x0a":
            return None
        request = request_of(datagram)
        self.requests.append(request)
        self.client = source
        apdu = request["apdu"]
        if apdu[0] >> 4 == 0:
            return [relay(request["dnet"], request["dadr"], bytes([0x20, apdu[2], apdu[3]]))]
        return None


class RoutedCovWireTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.wire = CovWire()
        self.router = RawPeer(self.wire.handle)
        self.addCleanup(self.router.close)
        self.target = rb.RoutedTarget(5, STATION, router=self.router.address)

    async def run_both(self, scenario):
        """`scenario` on a standalone client, then on an endpoint's."""
        for kind in ("standalone", "endpoint"):
            with self.subTest(client=kind):
                self.wire.requests.clear()
                if kind == "standalone":
                    async with rb.BACnetClient(interface="127.0.0.1", port=0,
                                               broadcast_address="127.0.0.1",
                                               apdu_timeout_ms=2000) as client:
                        await scenario(client)
                else:
                    endpoint = rb.BipEndpoint(NEAR, interface="127.0.0.1", port=0,
                                              broadcast_address="127.0.0.1",
                                              apdu_timeout_ms=2000)
                    await asyncio.wait_for(endpoint.start(), 10)
                    try:
                        await scenario(await endpoint.client())
                    finally:
                        await finish(endpoint.close)

    def assert_routed(self, request, service=SUBSCRIBE_COV):
        # DNET present, expecting a reply, normal priority, no SNET.
        self.assertEqual(request["control"], 0x24)
        self.assertEqual((request["dnet"], request["dadr"]), (5, STATION))
        self.assertEqual(request["hop_count"], 255)
        self.assertEqual(request["apdu"][0] >> 4, 0)  # Confirmed-Request
        self.assertEqual(request["apdu"][3], service)

    async def test_subscribe_hear_and_cancel_through_the_router(self):
        async def scenario(client):
            queue, task = await collect(client)
            try:
                await asyncio.wait_for(client.subscribe_cov(
                    self.target, PROCESS, AI_1, confirmed=False, lifetime=60), 5)
                [subscribe] = self.wire.requests
                self.assert_routed(subscribe)
                # The process, the object, unconfirmed, 60 s.
                self.assertEqual(subscribe["apdu"][4:],
                                 cancel_body(PROCESS) + b"\x29\x00\x39\x3c")

                # The device's notification comes back through the router,
                # naming the station; the client hears it.
                self.router.send(relay(5, STATION, notification(PROCESS, 21.5)),
                                 self.wire.client)
                heard = await asyncio.wait_for(queue.get(), 5)
                self.assertEqual((heard.source_network, heard.source_address),
                                 (5, STATION))
                self.assertEqual(heard.source_mac, mac(self.router.address))
                self.assertEqual(heard.subscriber_process_identifier, PROCESS)
                self.assertEqual(heard.monitored_object_identifier, AI_1)
                self.assertEqual(heard.initiating_device_identifier,
                                 rb.ObjectIdentifier(O.DEVICE, FAR))
                self.assertIn(rb.PropertyValue.real(21.5),
                              [item["value"] for item in heard.values])

                # The cancel goes to the same station, for the same process
                # and object, with no mode or lifetime.
                await asyncio.wait_for(client.unsubscribe_cov(self.target, PROCESS, AI_1), 5)
                subscribe, cancel = self.wire.requests
                self.assert_routed(cancel)
                self.assertEqual(cancel["apdu"][4:], cancel_body(PROCESS))
            finally:
                await stop(task)

        await self.run_both(scenario)

    async def test_property_subscriptions_go_to_the_station(self):
        async def scenario(client):
            await asyncio.wait_for(client.subscribe_cov_property(
                self.target, PROCESS, AI_1, PV, False, 60, cov_increment=0.5), 5)
            await asyncio.wait_for(client.unsubscribe_cov_property(
                self.target, PROCESS, AI_1, PV), 5)
            await asyncio.wait_for(client.subscribe_cov_property_multiple(
                self.target, PROCESS, [(AI_1, [(PV, None, None, False)])],
                issue_confirmed_notifications=False, max_notification_delay=5, lifetime=60), 5)
            subscribe, cancel, multiple = self.wire.requests
            for request, service in ((subscribe, SUBSCRIBE_COV_PROPERTY),
                                     (cancel, SUBSCRIBE_COV_PROPERTY),
                                     (multiple, SUBSCRIBE_COV_PROPERTY_MULTIPLE)):
                self.assert_routed(request, service)
            # The process, the object, unconfirmed, 60 s, Present_Value, 0.5.
            prop = b"\x4e\x09\x55\x4f"
            self.assertEqual(subscribe["apdu"][4:], cancel_body(PROCESS) + b"\x29\x00\x39\x3c"
                             + prop + b"\x5c" + struct.pack(">f", 0.5))
            # A cancel: the same process, object and property, nothing else.
            self.assertEqual(cancel["apdu"][4:], cancel_body(PROCESS) + prop)

            # Checked before anything is sent: a zero lifetime, and an
            # address that is neither a str nor a RoutedTarget.
            self.wire.requests.clear()
            with self.assertRaisesRegex(ValueError, "1 or more seconds"):
                client.subscribe_cov_property(self.target, PROCESS, AI_1, PV, False, 0)
            for call in (lambda to: client.subscribe_cov_property(to, PROCESS, AI_1, PV, False, 60),
                         lambda to: client.unsubscribe_cov_property(to, PROCESS, AI_1, PV),
                         lambda to: client.subscribe_cov_property_multiple(
                             to, PROCESS, [(AI_1, [(PV, None, None, False)])], False, 5, 60)):
                with self.assertRaisesRegex(TypeError, "str or a RoutedTarget, not int"):
                    call(7)
            self.assertEqual(self.wire.requests, [])

        await self.run_both(scenario)

    async def test_a_str_is_direct_and_a_wrong_type_or_group_router_raises(self):
        async def scenario(client):
            await asyncio.wait_for(client.subscribe_cov(
                self.router.address, PROCESS, AI_1, confirmed=True, lifetime=30), 5)
            await asyncio.wait_for(client.unsubscribe_cov(self.router.address, PROCESS, AI_1), 5)
            self.assertEqual([(r["control"], r["dnet"]) for r in self.wire.requests],
                             [(0x04, None), (0x04, None)])
            self.wire.requests.clear()
            for call in (lambda to: client.subscribe_cov(to, PROCESS, AI_1, False, 60),
                         lambda to: client.unsubscribe_cov(to, PROCESS, AI_1)):
                with self.assertRaisesRegex(TypeError, "str or a RoutedTarget, not int"):
                    call(7)
                for router in ("255.255.255.255:47808", "239.255.1.1:47808"):
                    with self.subTest(router=router), \
                            self.assertRaisesRegex(rb.BacnetError, "broadcast or group"):
                        await asyncio.wait_for(
                            call(rb.RoutedTarget(5, STATION, router=router)), 5)
            self.assertEqual(self.wire.requests, [])

        await self.run_both(scenario)


class LearnedCovRouteTests(unittest.IsolatedAsyncioTestCase):
    """A RoutedTarget with no router: the endpoint is a foreign device of a
    raw BBMD that is also the router, so its Who-Is shows on the wire."""

    async def asyncSetUp(self):
        self.who_is = []
        self.requests = []

        def handle(datagram, source):
            if datagram[:2] == b"\x81\x05":  # Register-Foreign-Device: accept
                return [bvll(0x00, b"\x00\x00")]
            if datagram[:2] == b"\x81\x09":  # a broadcast: answer a Who-Is to 5
                request = request_of(datagram, 0x09)
                if request["dnet"] == 5 and request["apdu"][:2] == b"\x10\x08":
                    self.who_is.append(request)
                    return [i_am(5, STATION, FAR)]
                return None
            if datagram[:2] != b"\x81\x0a":
                return None
            request = request_of(datagram)
            self.requests.append(request)
            apdu = request["apdu"]
            return [relay(request["dnet"], request["dadr"], bytes([0x20, apdu[2], apdu[3]]))]

        self.bbmd = RawPeer(handle)
        self.addCleanup(self.bbmd.close)
        self.endpoint = rb.BipEndpoint(
            NEAR + 30, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
            apdu_timeout_ms=2000, foreign_device=rb.ForeignDevice(self.bbmd.address, ttl=60))
        await asyncio.wait_for(self.endpoint.start(), 10)
        self.addAsyncCleanup(finish, self.endpoint.close)
        await accepted_registration(self.endpoint)
        self.role = await self.endpoint.client()

    async def test_the_subscription_learns_the_route_and_the_cancel_reuses_it(self):
        target = rb.RoutedTarget(5, STATION)
        await asyncio.wait_for(self.role.subscribe_cov(target, PROCESS, AI_1, False, 60), 10)
        await asyncio.wait_for(self.role.unsubscribe_cov(target, PROCESS, AI_1), 5)
        self.assertEqual(len(self.who_is), 1)
        self.assertEqual([(r["control"], r["dnet"], r["dadr"]) for r in self.requests],
                         [(0x24, 5, STATION)] * 2)
        self.assertEqual(self.requests[1]["apdu"][4:], cancel_body(PROCESS))

    async def test_a_silent_station_says_to_name_its_router(self):
        silent = rb.RoutedTarget(5, b"\x0b")
        with self.assertRaises(rb.BacnetLookupError) as caught:
            await asyncio.wait_for(self.role.subscribe_cov(silent, PROCESS, AI_1, False, 60), 10)
        for words in ("5:0x0B", "Who-Is on network 5", "router=", "discover_routers()"):
            self.assertIn(words, str(caught.exception))
        # The cancel is refused at once, with no second Who-Is.
        with self.assertRaisesRegex(rb.BacnetLookupError, "in the last 30 s"):
            await asyncio.wait_for(self.role.unsubscribe_cov(silent, PROCESS, AI_1), 1)
        self.assertEqual(len(self.who_is), 1)
        self.assertEqual(self.requests, [])


class RoutedCovRouterTests(unittest.IsolatedAsyncioTestCase):
    """Through a real BACnetRouter to a real server on network 5."""

    async def asyncSetUp(self):
        self.router = rb.BACnetRouter([
            rb.BipRouterPort(n, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                             bbmd=rb.Bbmd(accept_foreign_devices=True, alone_on_subnet=True))
            for n in (1, 5)])
        await asyncio.wait_for(self.router.start(), 10)
        self.addAsyncCleanup(finish, self.router.close)
        ports = await self.router.ports()
        self.router_address = ports[0].address
        self.far = rb.BACnetServer(FAR, interface="127.0.0.1", port=0,
                                   broadcast_address="127.0.0.1",
                                   foreign_device=rb.ForeignDevice(ports[1].address, ttl=60))
        self.far.add_analog_input(1, "Supply air", present_value=10.0)
        await asyncio.wait_for(self.far.start(), 10)
        self.addAsyncCleanup(finish, self.far.stop)
        await accepted_registration(self.far)
        self.far_address = await self.far.local_address()

    def target(self):
        return rb.RoutedTarget(5, self.far_address, router=self.router_address)

    async def counters(self):
        counters = await self.far.cov_counters()
        return {name: counters[name] for name in
                ("subscriptions_created", "subscriptions_active", "subscriptions_cancelled")}

    async def heard(self, queue, value, confirmed):
        """The next notification: back through the router, naming the server
        on network 5, with Present_Value `value`."""
        heard = await asyncio.wait_for(queue.get(), 5)
        self.assertEqual((heard.source_network, heard.source_address),
                         (5, mac(self.far_address)))
        self.assertEqual(heard.source_mac, mac(self.router_address))
        self.assertEqual(heard.delivery, "confirmed" if confirmed else "unconfirmed")
        self.assertEqual(heard.subscriber_process_identifier, PROCESS)
        self.assertIn(rb.PropertyValue.real(value),
                      [item["value"] for item in heard.values
                       if item["property_id"] == rb.PropertyIdentifier.PRESENT_VALUE])

    async def routed_subscription(self, client, confirmed):
        target = self.target()
        before = await self.counters()
        queue, task = await collect(client)
        try:
            await asyncio.wait_for(client.subscribe_cov(
                target, PROCESS, AI_1, confirmed=confirmed, lifetime=60), 10)
            await self.heard(queue, 10.0, confirmed)
            await self.far.set_present_value_local(AI_1, rb.PropertyValue.real(30.0))
            await self.heard(queue, 30.0, confirmed)

            # A renewal: the same target again is the same subscription, and
            # the device notifies once more.
            await asyncio.wait_for(client.subscribe_cov(
                target, PROCESS, AI_1, confirmed=confirmed, lifetime=60), 10)
            await self.heard(queue, 30.0, confirmed)
            after = await self.counters()
            self.assertEqual(after["subscriptions_created"], before["subscriptions_created"] + 1)
            self.assertEqual(after["subscriptions_active"], 1)

            # The cancel reaches the subscription the request made.
            await asyncio.wait_for(client.unsubscribe_cov(target, PROCESS, AI_1), 10)
            after = await self.counters()
            self.assertEqual(after["subscriptions_active"], 0)
            self.assertEqual(after["subscriptions_cancelled"],
                             before["subscriptions_cancelled"] + 1)
            await self.far.set_present_value_local(AI_1, rb.PropertyValue.real(10.0))
            with self.assertRaises(asyncio.TimeoutError):
                await asyncio.wait_for(queue.get(), 0.5)
        finally:
            await stop(task)

    async def test_routed_notifications_arrive_and_the_cancel_ends_them(self):
        async with rb.BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                                   apdu_timeout_ms=2000) as client:
            await self.routed_subscription(client, confirmed=False)
            await self.routed_subscription(client, confirmed=True)

    async def test_a_routed_property_subscription_notifies_renews_and_cancels(self):
        async with rb.BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                                   apdu_timeout_ms=2000) as client:
            target = self.target()
            before = await self.counters()
            queue, task = await collect(client)
            try:
                await asyncio.wait_for(client.subscribe_cov_property(
                    target, PROCESS, AI_1, PV, False, 60), 10)
                await self.heard(queue, 10.0, False)
                await self.far.set_present_value_local(AI_1, rb.PropertyValue.real(30.0))
                await self.heard(queue, 30.0, False)
                # A renewal: the same subscription again, which notifies once more.
                await asyncio.wait_for(client.subscribe_cov_property(
                    target, PROCESS, AI_1, PV, False, 60), 10)
                await self.heard(queue, 30.0, False)
                after = await self.counters()
                self.assertEqual(after["subscriptions_created"],
                                 before["subscriptions_created"] + 1)
                self.assertEqual(after["subscriptions_active"], 1)
                # The cancel to the same target ends it.
                await asyncio.wait_for(client.unsubscribe_cov_property(
                    target, PROCESS, AI_1, PV), 10)
                after = await self.counters()
                self.assertEqual(after["subscriptions_active"], 0)
                self.assertEqual(after["subscriptions_cancelled"],
                                 before["subscriptions_cancelled"] + 1)
                await self.far.set_present_value_local(AI_1, rb.PropertyValue.real(10.0))
                with self.assertRaises(asyncio.TimeoutError):
                    await asyncio.wait_for(queue.get(), 0.5)
            finally:
                await stop(task)

    async def heard_multiple(self, queue, process, value, confirmed):
        """The next notification: a COV-multiple one (#2654), back through the
        router, naming the server on network 5, with AI 1's Present_Value
        `value`, timestamped when `confirmed`."""
        heard = await asyncio.wait_for(queue.get(), 5)
        self.assertEqual(heard.kind, "multiple")
        self.assertEqual((heard.source_network, heard.source_address),
                         (5, mac(self.far_address)))
        self.assertEqual(heard.source_mac, mac(self.router_address))
        self.assertEqual(heard.delivery, "confirmed" if confirmed else "unconfirmed")
        self.assertEqual(heard.subscriber_process_identifier, process)
        [reported] = heard.objects
        self.assertEqual(reported["monitored_object_identifier"], AI_1)
        values = [item for item in reported["values"] if item["property_id"] == PV]
        self.assertIn(rb.PropertyValue.real(value), [item["value"] for item in values])
        self.assertEqual(heard.timestamp is not None, confirmed)
        self.assertEqual(all(item["time_of_change"] is not None for item in values), confirmed)

    async def test_a_routed_property_multiple_subscription_notifies_and_cancels(self):
        async with rb.BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                                   apdu_timeout_ms=2000) as client:
            target = self.target()
            queue, task = await collect(client)
            try:
                for process, confirmed in ((PROCESS + 1, True), (PROCESS + 2, False)):
                    with self.subTest(confirmed=confirmed):
                        # Confirmed and timestamped, or neither.
                        await asyncio.wait_for(client.subscribe_cov_property_multiple(
                            target, process, [(AI_1, [(PV, None, None, confirmed)])],
                            issue_confirmed_notifications=confirmed,
                            max_notification_delay=1, lifetime=60), 10)
                        await self.heard_multiple(queue, process, 10.0, confirmed)
                        await self.far.set_present_value_local(AI_1, rb.PropertyValue.real(30.0))
                        await self.heard_multiple(queue, process, 30.0, confirmed)
                        # The whole context's cancel, to the same target, ends it.
                        await asyncio.wait_for(client.subscribe_cov_property_multiple(
                            target, process, [], issue_confirmed_notifications=confirmed), 10)
                        await self.far.set_present_value_local(AI_1, rb.PropertyValue.real(10.0))
                        with self.assertRaises(asyncio.TimeoutError):
                            await asyncio.wait_for(queue.get(), 1)
            finally:
                await stop(task)

    async def test_a_cancel_by_another_address_leaves_the_subscription(self):
        # The server keeps the subscription under the address the request
        # came from: this client through the router. The same server asked
        # directly, by its own ip:port, sees the cancel from the client's
        # own address, finds nothing to cancel and says so with success; the
        # routed subscription stays until a cancel names the same target.
        async with rb.BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                                   apdu_timeout_ms=2000) as client:
            target = self.target()
            await asyncio.wait_for(client.subscribe_cov(target, PROCESS, AI_1, False, 60), 10)
            await asyncio.wait_for(client.unsubscribe_cov(self.far_address, PROCESS, AI_1), 10)
            self.assertEqual((await self.counters())["subscriptions_active"], 1)
            await asyncio.wait_for(client.unsubscribe_cov(target, PROCESS, AI_1), 10)
            self.assertEqual((await self.counters())["subscriptions_active"], 0)

if __name__ == "__main__":
    unittest.main()
