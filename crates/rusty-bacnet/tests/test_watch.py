"""Watching points from Python (#2561 part 3): ``Device.watch``.

``await device.watch(points)`` hands back a ``Watch``, an async iterator of
``WatchUpdate``s over a ``bacnet-ops`` watch hub of its own: each point is
read, then subscribed for COV and renewed before each lifetime ends, or
polled where the device won't subscribe it. The watch ends, its iterator
with it, on ``close()``, when it goes away, or when its client stops; its
subscriptions are cancelled then.

The tests run servers in this process, most behind a UDP relay that
records each confirmed request it passes, can refuse SubscribeCOV, can go
silent, and can be pointed at another server at the same address: the
device there restarting, or another device taking its place. Timing is
real (Python has no paused clock), so the tests use short lifetimes with
generous bounds and count requests rather than time them.
"""

from __future__ import annotations

import ast
import asyncio
import copy
import gc
import inspect
import pickle
import time
import unittest

import rusty_bacnet
from rusty_bacnet import (
    BacnetError,
    BacnetLookupError,
    BacnetStateError,
    BACnetClient,
    BACnetServer,
    Device,
    EngineeringUnits,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
    Value,
    Watch,
    WatchMove,
    WatchReason,
    WatchUpdate,
)

from test_inventory import Back, Relay
from test_relinquish import runtime_parameters, stub_class, stub_method, stub_parameters
from test_router_application import cleanup

DEVICE = 2604
OTHER = 2654
AI = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
AI2 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 2)

READ_PROPERTY = 12
SUBSCRIBE_COV = 5


def client(**keywords) -> BACnetClient:
    defaults = {"interface": "127.0.0.1", "port": 0, "broadcast_address": "127.0.0.1",
                "apdu_timeout_ms": 1000}
    return BACnetClient(**{**defaults, **keywords})


async def plant(instance: int = DEVICE, value: float = 20.0) -> BACnetServer:
    """A device with two analog inputs, AI 1 reading ``value``."""
    server = BACnetServer(instance, f"Watched {instance}", interface="127.0.0.1", port=0,
                          broadcast_address="127.0.0.1")
    server.add_analog_input(1, "Zone Temp", EngineeringUnits.DEGREES_CELSIUS, value)
    server.add_analog_input(2, "Supply Temp", EngineeringUnits.DEGREES_CELSIUS, 12.0)
    await server.start()
    return server


class CovRelay(Relay):
    """The inventory tests' relay, which can also refuse SubscribeCOV with
    a Reject for an unrecognized service, and be pointed at another
    server (``server``)."""

    def __init__(self, server: tuple[str, int]) -> None:
        super().__init__(server)
        self.refuse_cov = False

    def datagram_received(self, data: bytes, addr: tuple[str, int]) -> None:
        confirmed = len(data) > 9 and data[5] & 0x28 == 0 and data[6] >> 4 == 0
        if self.refuse_cov and confirmed and data[9] == SUBSCRIBE_COV:
            self.client = addr
            self.services.append(SUBSCRIBE_COV)
            reject = bytes([0x01, 0x00, 0x60, data[8], 9])
            assert self.front is not None
            self.front.sendto(b"\x81\x0a" + (len(reject) + 4).to_bytes(2, "big") + reject, addr)
            return
        super().datagram_received(data, addr)

    def point_at(self, address: str) -> None:
        host, port = address.rsplit(":", 1)
        self.server = (host, int(port))

    def count(self, service: int) -> int:
        return self.services.count(service)


async def relay_to(address: str) -> CovRelay:
    host, port = address.rsplit(":", 1)
    loop = asyncio.get_running_loop()
    relay = CovRelay((host, int(port)))
    relay.front, _ = await loop.create_datagram_endpoint(
        lambda: relay, local_addr=("127.0.0.1", 0))
    relay.back, _ = await loop.create_datagram_endpoint(
        lambda: Back(relay), local_addr=("127.0.0.1", 0))
    return relay


async def until(check, seconds: float = 10.0, what: str = "the condition") -> None:
    """Wait for ``check()`` (sync or async) to be true."""
    deadline = time.monotonic() + seconds
    while True:
        result = check()
        if inspect.isawaitable(result):
            result = await result
        if result:
            return
        if time.monotonic() > deadline:
            raise AssertionError(f"timed out waiting for {what}")
        await asyncio.sleep(0.02)


async def next_where(watch: Watch, test, seconds: float = 15.0) -> WatchUpdate:
    """The first update ``test`` accepts, failing past ``seconds``."""
    deadline = time.monotonic() + seconds
    while True:
        left = deadline - time.monotonic()
        if left <= 0:
            raise AssertionError("no such update came")
        update = await asyncio.wait_for(anext(watch), left)
        if test(update):
            return update


async def table_holds(c: BACnetClient, rows: set[tuple[int, str]]) -> bool:
    """Whether the discovery table has each (instance, address) of ``rows``."""
    held = {(d.object_identifier.instance, d.mac_address) for d in await c.discovered_devices()}
    return all((instance, mac_of(address)) in held for instance, address in rows)


def mac_of(address: str) -> bytes:
    """The B/IP MAC of ``address`` (``host:port``)."""
    host, port = address.rsplit(":", 1)
    return bytes(int(octet) for octet in host.split(".")) + int(port).to_bytes(2, "big")


def i_am(instance: int) -> bytes:
    """An I-Am of device ``instance`` in an Original-Unicast-NPDU, for the
    relay to send to the client's own address. Not an
    Original-Broadcast-NPDU: that must arrive at the broadcast address, and
    Windows reports a unicast delivery as one (``MSG_BCAST`` unset), so the
    client rightly drops it there, while hosts that report nothing take the
    loopback broadcast address, the client's own here, as a broadcast."""
    apdu = (bytes([0x10, 0x00, 0xC4]) + ((8 << 22) | instance).to_bytes(4, "big")
            + bytes([0x22, 0x05, 0xC4, 0x91, 0x03, 0x21, 0x2A]))
    npdu = bytes([0x01, 0x00]) + apdu
    return bytes([0x81, 0x0A]) + (len(npdu) + 4).to_bytes(2, "big") + npdu


def subscribed(update: WatchUpdate) -> bool:
    return update.state == "subscribed" and update.value is not None


def valued(value: float):
    """A test for an update carrying ``value``."""
    return lambda update: update.value is not None and update.value.value.value == value


async def drain(watch: Watch, quiet: float = 1.0) -> None:
    """Take updates until none comes for ``quiet`` seconds."""
    while True:
        try:
            await asyncio.wait_for(anext(watch), quiet)
        except TimeoutError:
            return


async def active(server: BACnetServer) -> int:
    return (await server.cov_counters())["subscriptions_active"]


class Lab(unittest.IsolatedAsyncioTestCase):
    """A device behind a relay."""

    async def asyncSetUp(self) -> None:
        self.server = await plant()
        self.addAsyncCleanup(self.stop_server)
        self.relay = await relay_to(await self.server.local_address())
        self.addCleanup(self.relay.close)
        self.address = self.relay.address()

    async def stop_server(self) -> None:
        await cleanup(self.server.stop)

    async def restart_as(self, instance: int, value: float) -> None:
        """Stop the device and put another at its address: the same one
        restarted, or a different one."""
        await self.server.stop()
        self.server = await plant(instance, value)
        self.relay.point_at(await self.server.local_address())


class UpdateTests(Lab):
    async def test_a_changed_value_comes_as_a_cov_update(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1"])
            self.assertIsInstance(watch, Watch)
            # The first read's value, or the subscription's first
            # notification when that is newer, then subscribed.
            await next_where(watch, subscribed)
            await self.server.set_present_value_local(AI, PropertyValue.real(23.5))
            update = await next_where(watch, valued(23.5))
            self.assertEqual(self.relay.count(SUBSCRIBE_COV), 1)
            await watch.close()
            # The cancel is a SubscribeCOV without a lifetime.
            self.assertEqual(self.relay.count(SUBSCRIBE_COV), 2)
            self.assertEqual(await active(self.server), 0)
        self.assertIsInstance(update, WatchUpdate)
        self.assertEqual(update.address, self.address)
        self.assertEqual(update.object_id, AI)
        self.assertEqual(update.property_id, PropertyIdentifier.PRESENT_VALUE)
        self.assertIsNone(update.array_index)
        self.assertEqual(update.state, "subscribed")
        self.assertIsNone(update.why)
        self.assertIsInstance(update.value, Value)
        self.assertEqual(update.value.value.value, 23.5)
        # The first read had the units, so the text names them.
        self.assertEqual(update.value.text, "23.5 °C")
        self.assertIsNone(update.error)
        # Status_Flags as an inventory's Point has them.
        self.assertIsInstance(update.status_flags, Value)
        self.assertEqual(update.status_flags.value.value.names, [])
        self.assertLess(abs(update.timestamp - time.time()), 60)
        self.assertEqual(update.cov_support, "accepted")
        self.assertEqual(repr(update),
                         "WatchUpdate(analog-input 1, present-value, subscribed, "
                         "value='23.5 °C', via=cov)")

    async def test_a_property_point_and_the_watch_getters(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch([("ai:2", "status-flags"), AI, "ai:1 pv"],
                                    lifetime=60, poll_interval=2)
            self.assertEqual(watch.address, self.address)
            # Each point once, in the order named.
            self.assertEqual(watch.points, (
                (AI2, PropertyIdentifier.STATUS_FLAGS, None),
                (AI, PropertyIdentifier.PRESENT_VALUE, None),
            ))
            self.assertEqual(len(watch), 2)
            self.assertEqual((watch.lifetime, watch.poll_interval), (60, 2.0))
            self.assertFalse(watch.closed)
            self.assertEqual(repr(watch), f"Watch({self.address}, points=2, open)")
            flags = await next_where(
                watch, lambda u: u.object_id == AI2 and u.value is not None)
            self.assertEqual(flags.property_id, PropertyIdentifier.STATUS_FLAGS)
            await watch.close()
            self.assertTrue(watch.closed)
            self.assertEqual(repr(watch), f"Watch({self.address}, points=2, closed)")

    async def test_a_device_refusing_cov_is_polled(self) -> None:
        self.relay.refuse_cov = True
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1"], poll_interval=1)
            polled = await next_where(watch, lambda u: u.state == "polled")
            self.assertIsInstance(polled.why, WatchReason)
            self.assertEqual(polled.why.kind, "refused")
            self.assertIn("reject", polled.why.error.lower())
            self.assertIn("refused", str(polled.why))
            self.assertIsNone(polled.why.to)
            self.assertEqual(polled.cov_support, "refused")
            await self.server.set_present_value_local(AI, PropertyValue.real(31.0))
            update = await next_where(watch, valued(31.0))
            await watch.close()
        self.assertEqual(update.via, "poll")
        self.assertEqual(update.state, "polled")
        # The refusal is remembered: asked once.
        self.assertEqual(self.relay.count(SUBSCRIBE_COV), 1)


class LifetimeTests(Lab):
    async def test_subscriptions_are_renewed_before_they_lapse(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1"], lifetime=6)
            await next_where(watch, subscribed)
            # Renewed halfway through each lifetime: three requests in all
            # well within the bound, and the device never let the
            # subscription lapse.
            await until(lambda: self.relay.count(SUBSCRIBE_COV) >= 3, 20, "renewals")
            self.assertEqual(await active(self.server), 1)
            self.assertEqual((await self.server.cov_counters())["subscriptions_purged"], 0)
            await self.server.set_present_value_local(AI, PropertyValue.real(26.0))
            update = await next_where(watch, valued(26.0))
            self.assertEqual(update.via, "cov")
            await watch.close()

    async def test_updates_resume_after_the_device_restarts(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1"], lifetime=4, poll_interval=1)
            await next_where(watch, subscribed)
            await self.restart_as(DEVICE, 44.0)
            update = await next_where(watch, valued(44.0), 20)
            self.assertIn(update.via, ("cov", "poll", "read"))
            self.assertNotEqual(update.state, "gone")
            await watch.close()

    async def test_another_device_at_the_address_is_reported_not_taken(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1"], lifetime=4, poll_interval=1)
            await next_where(watch, subscribed)
            # Once subscribed, the hub asks the device which it is: an answer
            # naming it confirms the point's device.
            subscribe = self.relay.services.index(SUBSCRIBE_COV)
            await until(lambda: READ_PROPERTY in self.relay.services[subscribe:], 10,
                        "the identity read")
            await self.restart_as(OTHER, 999.0)
            seen = []

            def gone(update: WatchUpdate) -> bool:
                seen.append(update)
                return update.state == "gone"

            replaced = await next_where(watch, gone, 30)
            self.assertEqual(replaced.why.kind, "replaced")
            self.assertEqual((replaced.why.watched, replaced.why.found), (DEVICE, OTHER))
            self.assertEqual(str(replaced.why),
                             f"device {OTHER} answers at {self.address} now, "
                             f"not device {DEVICE}")
            # Nothing more comes for the point, the new device's values least
            # of all.
            with self.assertRaises(TimeoutError):
                await asyncio.wait_for(anext(watch), 2)
            await watch.close()
        values = [u.value.value.value for u in seen if u.value is not None]
        self.assertNotIn(999.0, values)


class FollowTests(Lab):
    async def test_a_watch_of_an_instance_follows_its_device(self) -> None:
        # The same device answering at its own address too: where it goes
        # once the relay's address falls silent and the table forgets it.
        moved = await plant(DEVICE, 55.0)
        self.addAsyncCleanup(cleanup, moved.stop)
        new = await moved.local_address()
        async with client() as c:
            await c.discover(scope=self.address, wait=0.3)
            dev = await c.device(DEVICE)
            self.assertTrue(dev.follows)
            watch = await dev.watch(["ai:1"], lifetime=4, poll_interval=1)
            self.assertTrue(watch.follows)
            first = await next_where(watch, subscribed)
            self.assertEqual((first.address, first.instance, first.moved),
                             (self.address, DEVICE, None))
            self.relay.passing = 0
            await c.clear_devices()
            await c.discover(scope=new, wait=0.3)
            went = await next_where(watch, lambda u: u.moved is not None, 30)
            self.assertIsInstance(went.moved, WatchMove)
            self.assertEqual((went.moved.from_, went.moved.to, went.address),
                             (self.address, new, new))
            self.assertLess(abs(went.moved.timestamp - time.time()), 60)
            self.assertEqual(went.moved.count, 1)
            self.assertEqual(repr(went.moved), f"WatchMove({self.address} -> {new})")
            with self.assertRaises(TypeError):
                pickle.dumps(went.moved)
            # The same point, now from the device's own address.
            self.assertEqual((went.object_id, went.instance), (AI, DEVICE))
            await next_where(watch, lambda u: subscribed(u) and u.address == new, 30)
            await moved.set_present_value_local(AI, PropertyValue.real(56.5))
            update = await next_where(watch, valued(56.5))
            self.assertEqual((update.address, update.moved.to), (new, new))
            # The watch took the move after its own request at the relay's
            # address went unanswered, so the handle stays until a read of
            # its own asks there and finds the device moved.
            self.assertEqual(dev.address, self.address)
            self.assertEqual((await dev.read("ai:1")).data, 56.5)
            self.assertEqual(dev.address, new)
            await watch.close()
            self.assertEqual(await active(moved), 0)

    async def test_a_write_never_moves_a_following_handle(self) -> None:
        moved = await plant(DEVICE, 55.0)
        self.addAsyncCleanup(cleanup, moved.stop)
        new = await moved.local_address()
        async with client() as c:
            await c.discover(scope=self.address, wait=0.3)
            dev = await c.device(DEVICE)
            # The table: another device at the relay's address, which then
            # answers nothing, and the device at its own. Until the table
            # holds both rows, the handle's own row still names the device,
            # and the write would rightly go there.
            self.relay.answer(i_am(OTHER))
            await c.discover(scope=new, wait=0.3)
            await until(lambda: table_holds(c, {(OTHER, self.address), (DEVICE, new)}),
                        10, "both rows in the discovery table")
            self.relay.passing = 0
            with self.assertRaises(BacnetLookupError) as caught:
                await dev.write("ai:1", "out-of-service", "true")
            error = caught.exception
            self.assertEqual((error.target, error.instance), (str(DEVICE), DEVICE))
            self.assertEqual(error.at, (0, mac_of(self.address)))
            self.assertEqual(error.candidate, (0, mac_of(new)))
            self.assertEqual(error.candidates, [], "a hint is no choice")
            self.assertIn(f"isn't confirmed at {self.address}", str(error))
            self.assertIn(f"the discovery table has it at {new}", str(error))
            self.assertEqual(dev.address, self.address, "a write moves no handle")
            # Nothing was written at the device's own address either.
            there = await c.device(new)
            self.assertIs((await there.read("ai:1", "out-of-service")).data, False)

    async def test_a_watch_of_an_address_stays_there(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1"])
            self.assertFalse(watch.follows)
            update = await next_where(watch, subscribed)
            self.assertEqual((update.address, update.instance, update.moved),
                             (self.address, DEVICE, None))
            await watch.close()


class EndTests(Lab):
    async def test_close_and_async_with_cancel_the_subscription(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            async with await dev.watch(["ai:1"]) as watch:
                await next_where(watch, subscribed)
                self.assertEqual(await active(self.server), 1)
            # Leaving `async with` closed it, its cancel answered.
            self.assertTrue(watch.closed)
            self.assertEqual(await active(self.server), 0)
            with self.assertRaisesRegex(StopAsyncIteration, "it was closed"):
                await anext(watch)
            await watch.close()
            # The client goes on.
            other = await dev.watch(["ai:1"])
            await next_where(other, subscribed)
            await other.close()

    async def test_a_dropped_watch_cancels_and_renews_no_more(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1"], lifetime=2)
            await next_where(watch, subscribed)
            self.assertEqual(await active(self.server), 1)
            del watch
            gc.collect()
            await until(self._none_active, 10, "the cancel")
            asked = self.relay.count(SUBSCRIBE_COV)
            # Longer than a lifetime: a renewal would have gone by now.
            await asyncio.sleep(3)
            self.assertEqual(self.relay.count(SUBSCRIBE_COV), asked)
            self.assertEqual(await active(self.server), 0)

    async def _none_active(self) -> bool:
        return await active(self.server) == 0

    async def test_stop_ends_a_live_watch(self) -> None:
        c = client()
        await c.__aenter__()
        self.addAsyncCleanup(cleanup, c.stop)
        dev = await c.device(self.address)
        watch = await dev.watch(["ai:1"])
        await next_where(watch, subscribed)
        got = []

        async def read() -> None:
            async for update in watch:
                got.append(update)

        reader = asyncio.ensure_future(read())
        await asyncio.sleep(0.1)
        await asyncio.wait_for(c.stop(), 15)
        # The `async for` ended: its __anext__ raised StopAsyncIteration.
        await asyncio.wait_for(reader, 5)
        self.assertTrue(watch.closed)
        with self.assertRaisesRegex(StopAsyncIteration, "the client stopped"):
            await anext(watch)
        await watch.close()
        self.assertEqual(await active(self.server), 0)
        # A restarted client doesn't revive the watch; a new one works.
        async with c:
            with self.assertRaises(StopAsyncIteration):
                await anext(watch)
            again = await dev.watch(["ai:1"])
            await next_where(again, subscribed)

    async def test_stop_returns_with_a_watch_of_a_silent_device(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1"], lifetime=60)
            await next_where(watch, subscribed)
            self.relay.passing = 0
            started = time.monotonic()
        # The cancel went unanswered: the stop waited a bounded time, 5 s
        # for the cancel and up to 2 s for the hub to let go.
        self.assertLess(time.monotonic() - started, 15)
        self.assertTrue(watch.closed)


class LimitTests(unittest.IsolatedAsyncioTestCase):
    async def test_equal_settings_share_a_hub_and_256_settings_fill_the_client(self) -> None:
        server = await plant()
        self.addAsyncCleanup(cleanup, server.stop)
        async with client() as c:
            dev = await c.device(await server.local_address())
            # Watches of equal settings share one hub, however many.
            same = [await dev.watch(["ai:1"], subscribe=False) for _ in range(300)]
            # Each other set of settings takes a hub of its own: 255 more
            # fill the client's 256 ranges of subscriber process identifiers.
            others = [await dev.watch(["ai:1"], subscribe=False, poll_interval=3000 + n)
                      for n in range(255)]
            with self.assertRaisesRegex(RuntimeError, "256 watches with different settings"):
                await dev.watch(["ai:1"], subscribe=False, poll_interval=2000)
            # Equal settings still join a hub that runs.
            joined = await dev.watch(["ai:1"], subscribe=False, poll_interval=3000)
            await joined.close()
            # A closed watch frees its hub once it was the hub's last, its
            # scheduler's task ended a moment after the close.
            await others.pop().close()
            deadline = time.monotonic() + 5
            while True:
                try:
                    fresh = await dev.watch(["ai:1"], subscribe=False, poll_interval=2000)
                    break
                except RuntimeError:
                    if time.monotonic() > deadline:
                        raise
                    await asyncio.sleep(0.05)
            await fresh.close()
            for watch in same + others:
                await watch.close()


class StepTests(Lab):
    async def test_a_cancelled_step_loses_no_update(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1"])
            await next_where(watch, subscribed)
            await drain(watch)
            for _ in range(3):
                with self.assertRaises(TimeoutError):
                    await asyncio.wait_for(anext(watch), 0.1)
            await self.server.set_present_value_local(AI, PropertyValue.real(27.0))
            update = await asyncio.wait_for(anext(watch), 10)
            self.assertEqual(update.value.value.value, 27.0)
            await watch.close()

    async def test_two_tasks_on_one_watch_get_one_update_each(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1", "ai:2"])
            await next_where(watch, lambda u: u.object_id == AI and subscribed(u))
            await next_where(watch, lambda u: u.object_id == AI2 and subscribed(u))
            # Quiet then: each change makes one update.
            await drain(watch)
            await self.server.set_present_value_local(AI, PropertyValue.real(1.0))
            await self.server.set_present_value_local(AI2, PropertyValue.real(2.0))
            pair = await asyncio.wait_for(asyncio.gather(anext(watch), anext(watch)), 10)
            self.assertEqual({(u.object_id.instance, u.value.value.value) for u in pair},
                             {(1, 1.0), (2, 2.0)})
            await watch.close()


class ArgumentTests(Lab):
    async def test_bad_arguments_raise_when_the_call_is_made(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            for points, error, text in (
                ("ai:1", TypeError, "put one in a list"),
                (b"ai:1", TypeError, "not bytes: put one in a list"),
                (bytearray(b"ai:1"), TypeError, "not bytearray: put one in a list"),
                ([], ValueError, "points is empty"),
                ([1.5], TypeError, ""),
                (["zz:1"], ValueError, ""),
                (["ai:1 all"], ValueError, "names many properties"),
                (["ai:%d" % n for n in range(4097)], ValueError, "at most 4096"),
            ):
                with self.subTest(points=str(points)[:40]):
                    with self.assertRaisesRegex(error, text):
                        dev.watch(points)
            for keywords, error, text in (
                ({"lifetime": 0}, ValueError, "lifetime is 1 to"),
                ({"lifetime": -5}, ValueError, "lifetime is 1 to"),
                ({"lifetime": True}, TypeError, "int of seconds"),
                ({"lifetime": 1.5}, TypeError, "int of seconds"),
                ({"poll_interval": 0.5}, ValueError, "poll_interval is 1"),
                ({"poll_interval": 3601}, ValueError, "poll_interval is 1"),
                ({"poll_interval": "1"}, TypeError, "number of seconds"),
                ({"poll_interval": False}, TypeError, "number of seconds"),
                # Eight hours at most: the longest every device must take.
                ({"lifetime": 28801}, ValueError, "lifetime is 1 to 28800 seconds"),
                ({"lifetime": 2**32}, ValueError, "lifetime is 1 to 28800 seconds"),
                ({"subscribe": 1}, TypeError, "subscribe is a bool"),
                ({"subscribe": "no"}, TypeError, "subscribe is a bool"),
            ):
                with self.subTest(keywords=keywords):
                    with self.assertRaisesRegex(error, text):
                        dev.watch(["ai:1"], **keywords)
            with self.assertRaises(TypeError):
                dev.watch(["ai:1"], 300)  # keyword only
            # Nothing was sent for any of them.
            self.assertEqual(self.relay.count(SUBSCRIBE_COV), 0)

    def test_the_client_refuses_watch_settings_when_made(self) -> None:
        for keywords, error, text in (
            ({"watch_lifetime": 0}, ValueError, "watch_lifetime is 1 to 28800"),
            ({"watch_lifetime": 28801}, ValueError, "watch_lifetime is 1 to 28800"),
            ({"watch_lifetime": 1.5}, TypeError, "watch_lifetime is an int"),
            ({"watch_poll_interval": 0.5}, ValueError, "watch_poll_interval is 1"),
            ({"watch_poll_interval": True}, TypeError, "watch_poll_interval is a number"),
            ({"watch_subscribe": 0}, TypeError, "watch_subscribe is a bool"),
        ):
            with self.subTest(keywords=keywords):
                with self.assertRaisesRegex(error, text):
                    client(**keywords)
        c = client()
        self.assertEqual((c.watch_lifetime, c.watch_poll_interval, c.watch_subscribe),
                         (300, 10.0, True))
        parameters = inspect.signature(BACnetClient).parameters
        self.assertEqual([parameters[name].default for name in
                          ("watch_lifetime", "watch_poll_interval", "watch_subscribe")],
                         [300, 10.0, True])
        c = client(watch_lifetime=28800, watch_poll_interval=2.5, watch_subscribe=False)
        self.assertEqual((c.watch_lifetime, c.watch_poll_interval, c.watch_subscribe),
                         (28800, 2.5, False))

    async def test_a_watch_and_its_updates_refuse_copy_and_pickle(self) -> None:
        self.relay.refuse_cov = True
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1"], poll_interval=1)
            polled = await next_where(watch, lambda u: u.why is not None)
            for item in (watch, polled, polled.why):
                with self.subTest(item=type(item).__name__):
                    with self.assertRaises(TypeError):
                        copy.copy(item)
                    with self.assertRaises(TypeError):
                        pickle.dumps(item)
            # A value copies and pickles.
            self.assertEqual(copy.copy(polled.value), polled.value)
            await watch.close()

    async def test_after_stop_a_watch_call_raises(self) -> None:
        c = client()
        async with c:
            dev = await c.device(self.address)
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await dev.watch(["ai:1"])


class ChangeTests(Lab):
    """Adding and removing points on a live watch, and its settings
    (#2603)."""

    async def test_add_and_remove_while_another_task_iterates(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1"])
            await next_where(watch, subscribed)
            removed = False
            late = []

            async def read() -> None:
                while True:
                    # Whether the removal had returned before this step began.
                    after = removed
                    update = await anext(watch)
                    if after and update.object_id == AI:
                        late.append(update)
                    if update.object_id == AI2 and update.value is not None \
                            and update.value.value.value == -1.0:
                        return

            reader = asyncio.ensure_future(read())
            self.assertEqual(await watch.add(["ai:2", "ai:1"]), 1, "ai:1 is held already")
            self.assertEqual(watch.points, (
                (AI, PropertyIdentifier.PRESENT_VALUE, None),
                (AI2, PropertyIdentifier.PRESENT_VALUE, None),
            ))
            self.assertEqual(len(watch), 2)
            await until(lambda: self.relay.count(SUBSCRIBE_COV) >= 2, 10, "ai:2 subscribed")
            for step in range(5):
                await self.server.set_present_value_local(AI, PropertyValue.real(step))
                await self.server.set_present_value_local(AI2, PropertyValue.real(step))
            self.assertEqual(await watch.remove(["ai:1"]), 1)
            removed = True
            self.assertEqual(await watch.remove(["ai:1"]), 0, "held no more")
            self.assertEqual(watch.points, ((AI2, PropertyIdentifier.PRESENT_VALUE, None),))
            for step in range(5):
                await self.server.set_present_value_local(AI, PropertyValue.real(50 + step))
            await self.server.set_present_value_local(AI2, PropertyValue.real(-1.0))
            await asyncio.wait_for(reader, 15)
            self.assertEqual(late, [], "an update of a removed point")
            # ai:1's subscription was cancelled, ai:2's stays.
            await until(lambda: self.relay.count(SUBSCRIBE_COV) >= 3, 10, "the cancel")
            self.assertEqual(await active(self.server), 1)
            await watch.close()
            # An ended watch is a state (#2819): BacnetStateError, which
            # both RuntimeError and BacnetError catch, its text unchanged.
            for change, points in ((watch.add, ["ai:1"]), (watch.remove, ["ai:2"])):
                for catch in (RuntimeError, BacnetError):
                    with self.subTest(change=change.__name__, caught_as=catch.__name__):
                        try:
                            await change(points)
                        except catch as error:
                            self.assertIsInstance(error, BacnetStateError)
                            self.assertEqual(error.state, "closed")
                            self.assertEqual(str(error), "the watch ended: it was closed")
                        else:
                            self.fail("an ended watch took a change")

    async def test_add_and_remove_take_effect_in_the_order_called(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1"])
            await next_where(watch, subscribed)
            # Neither awaited before the other is called: each acts when called.
            added = watch.add(["ai:2"])
            self.assertEqual(len(watch), 2, "added before the await")
            removed = watch.remove(["ai:2"])
            self.assertEqual((await added, await removed), (1, 1))
            self.assertEqual(watch.points, ((AI, PropertyIdentifier.PRESENT_VALUE, None),))
            self.assertEqual(len(watch), 1)
            await self.server.set_present_value_local(AI2, PropertyValue.real(5.0))
            await self.server.set_present_value_local(AI, PropertyValue.real(6.0))
            update = await next_where(watch, valued(6.0))
            self.assertEqual(update.object_id, AI)
            with self.assertRaises(TimeoutError):
                while True:
                    late = await asyncio.wait_for(anext(watch), 1)
                    self.assertNotEqual(late.object_id, AI2)
            await watch.close()
            # An ended watch refuses either change when it is called, not
            # when it is awaited.
            for call in (watch.add, watch.remove):
                with self.subTest(call=call.__name__):
                    with self.assertRaisesRegex(RuntimeError, "it was closed"):
                        call(["ai:2"])

    async def test_add_and_remove_refuse_bad_points_when_called(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1"])
            for points, error, text in (
                ("ai:2", TypeError, "put one in a list"),
                (b"ai:2", TypeError, "not bytes: put one in a list"),
                (bytearray(b"ai:2"), TypeError, "not bytearray: put one in a list"),
                ([], ValueError, "points is empty"),
                (["ai:1 all"], ValueError, "names many properties"),
            ):
                for call in (watch.add, watch.remove):
                    with self.subTest(call=call.__name__, points=str(points)):
                        with self.assertRaisesRegex(error, text):
                            call(points)
            await watch.close()

    async def test_a_point_removed_and_added_again_is_read_afresh(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1"])
            await next_where(watch, subscribed)
            await watch.remove([AI])
            await until(self._none_active, 10, "the cancel")
            await self.server.set_present_value_local(AI, PropertyValue.real(61.0))
            await watch.add([AI])
            first = await asyncio.wait_for(anext(watch), 10)
            self.assertEqual(first.value.value.value, 61.0)
            self.assertIn(first.via, ("read", "cov"))
            await next_where(watch, subscribed)
            await watch.close()

    async def _none_active(self) -> bool:
        return await active(self.server) == 0

    async def test_polling_only_sends_no_subscription(self) -> None:
        async with client(watch_subscribe=False, watch_poll_interval=1) as c:
            dev = await c.device(self.address)
            watch = await dev.watch(["ai:1"])
            self.assertEqual((watch.subscribe, watch.poll_interval), (False, 1.0))
            polled = await next_where(watch, lambda u: u.state == "polled")
            self.assertEqual(polled.why.kind, "poll-only")
            self.assertIsNone(polled.why.error)
            await self.server.set_present_value_local(AI, PropertyValue.real(36.0))
            update = await next_where(watch, valued(36.0))
            self.assertEqual(update.via, "poll")
            # This watch subscribes, whatever the client's default.
            other = await dev.watch(["ai:2"], subscribe=True)
            self.assertTrue(other.subscribe)
            await next_where(other, subscribed)
            await other.close()
            await watch.close()
        self.assertEqual(self.relay.count(SUBSCRIBE_COV), 2, "ai:2's and its cancel")

    async def test_watches_of_equal_settings_share_a_subscription(self) -> None:
        async with client(watch_lifetime=120) as c:
            dev = await c.device(self.address)
            first = await dev.watch(["ai:1"])
            second = await dev.watch(["ai:1"], lifetime=120)
            self.assertEqual((first.lifetime, second.lifetime), (120, 120))
            await next_where(first, subscribed)
            await next_where(second, subscribed)
            self.assertEqual(self.relay.count(SUBSCRIBE_COV), 1, "one subscription")
            self.assertEqual(await active(self.server), 1)
            # Closing one leaves the other's.
            await first.close()
            self.assertEqual(await active(self.server), 1)
            await self.server.set_present_value_local(AI, PropertyValue.real(44.0))
            await next_where(second, valued(44.0))
            await second.close()
            self.assertEqual(await active(self.server), 0)


class StubParityTests(unittest.TestCase):
    CLASSES = ("Watch", "WatchUpdate", "WatchReason", "WatchMove")

    def test_every_public_member_matches_the_stub(self) -> None:
        for name in self.CLASSES:
            runtime = getattr(rusty_bacnet, name)
            stubbed = {node.name: node for node in stub_class(name).body
                       if isinstance(node, ast.FunctionDef)}
            public = {member for member in vars(runtime) if not member.startswith("_")}
            with self.subTest(cls=name):
                self.assertEqual(public, {n for n in stubbed if not n.startswith("_")})
            for member in sorted(public):
                attribute = inspect.getattr_static(runtime, member)
                if not callable(attribute):
                    continue  # a getter
                with self.subTest(cls=name, method=member):
                    self.assertEqual(runtime_parameters(getattr(runtime, member)),
                                     stub_parameters(stubbed[member]))

    def test_the_device_call_matches_the_stub(self) -> None:
        runtime = runtime_parameters(Device.watch)
        self.assertEqual([n for n, _ in runtime],
                         ["points", "lifetime", "poll_interval", "subscribe"])
        self.assertEqual(runtime, stub_parameters(stub_method("Device", "watch")))
        defaults = {n: p.default for n, p in inspect.signature(Device.watch).parameters.items()}
        # None: the client's watch_* settings.
        self.assertEqual(
            (defaults["lifetime"], defaults["poll_interval"], defaults["subscribe"]),
            (None, None, None))
        # On the handle only: the client has no second spelling.
        self.assertFalse(hasattr(BACnetClient, "watch"))


if __name__ == "__main__":
    unittest.main()
