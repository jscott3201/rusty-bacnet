"""A device's points from Python (#2561 part 3): ``Device.inventory`` and
``Device.points``.

``inventory()`` reads the device's summary, its Object_List and then its
objects in batches, into an ``Inventory`` of ``Point`` records; a device
that goes silent part way leaves the records read and says why in
``stopped``. ``points()`` reads the Object_List, then hands the records out
as an async iterator, one batch of objects a refill, so a large device's
first points come after two requests; a device that stops answering raises
from the iterator after the records read before it.

The tests run a server in this process and reach it through a UDP relay
that records each confirmed request it passes, and can go silent, answer
late or refuse ReadPropertyMultiple.
"""

from __future__ import annotations

import ast
import asyncio
import copy
import inspect
import pickle
import unittest

import rusty_bacnet
from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetRejectError,
    BacnetTimeoutError,
    Device,
    DeviceSummary,
    EngineeringUnits,
    Inventory,
    ObjectIdentifier,
    ObjectType,
    Point,
    PointIterator,
    PropertyIdentifier,
    PropertyValue,
    Value,
)

from test_relinquish import runtime_parameters, stub_class, stub_parameters
from test_router_application import cleanup

DEVICE = 2563
AI = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
AO = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 2)
BV = ObjectIdentifier(ObjectType.BINARY_VALUE, 3)
MSV = ObjectIdentifier(ObjectType.MULTI_STATE_VALUE, 4)
AV = ObjectIdentifier(ObjectType.ANALOG_VALUE, 5)
OWN = ObjectIdentifier(ObjectType.DEVICE, DEVICE)

READ_PROPERTY = 12
READ_PROPERTY_MULTIPLE = 14


class Relay(asyncio.DatagramProtocol):
    """Passes datagrams between one client and the server, recording each
    confirmed request's service by invoke ID. Requests after ``passing``
    are dropped; with ``refuse_rpm`` a ReadPropertyMultiple is answered
    with a Reject for an unrecognized service, and with ``refuse_list`` a
    ReadProperty of the whole Object_List (``"whole"``) or of any of it
    (``"all"``) too; ``delay`` holds each answer back that many seconds. A
    retransmission is passed or dropped as its first copy was, and not
    recorded again."""

    def __init__(self, server: tuple[str, int]) -> None:
        self.server = server
        self.passing = 1_000_000
        self.refuse_rpm = False
        self.refuse_list: str | None = None
        self.delay = 0.0
        self.seen: dict[int, bool] = {}
        self.services: list[int] = []
        self.client: tuple[str, int] | None = None
        self.front: asyncio.DatagramTransport | None = None
        self.back: asyncio.DatagramTransport | None = None

    def datagram_received(self, data: bytes, addr: tuple[str, int]) -> None:
        self.client = addr
        # BVLL (4 octets), the NPDU's version and control, then the APDU: a
        # local confirmed request's invoke ID is its third octet and its
        # service the fourth.
        if len(data) > 9 and data[5] & 0x28 == 0 and data[6] >> 4 == 0:
            invoke_id, service = data[8], data[9]
            if invoke_id not in self.seen:
                self.seen[invoke_id] = len(self.services) < self.passing
                self.services.append(service)
            if not self.seen[invoke_id]:
                return
            # A ReadProperty's object identifier (context 0, five octets),
            # then Object_List (context 1, 76), then an index or nothing.
            body = data[10:]
            listed = service == READ_PROPERTY and body[5:7] == b"\x19\x4c"
            refused_list = listed and (self.refuse_list == "all"
                                       or (self.refuse_list == "whole" and len(body) == 7))
            if (self.refuse_rpm and service == READ_PROPERTY_MULTIPLE) or refused_list:
                reject = bytes([0x01, 0x00, 0x60, invoke_id, 9])
                assert self.front is not None
                self.front.sendto(b"\x81\x0a" + (len(reject) + 4).to_bytes(2, "big") + reject,
                                  addr)
                return
        assert self.back is not None
        self.back.sendto(data, self.server)

    def answer(self, data: bytes) -> None:
        if self.client is None or self.front is None:
            return
        if self.delay:
            asyncio.get_running_loop().call_later(self.delay, self.front.sendto, data,
                                                  self.client)
        else:
            self.front.sendto(data, self.client)

    def forget(self) -> None:
        """Pass everything from now on; requests already dropped stay so."""
        self.passing = 1_000_000
        self.seen = {invoke: True for invoke in self.seen}

    def address(self) -> str:
        assert self.front is not None
        host, port = self.front.get_extra_info("sockname")[:2]
        return f"{host}:{port}"

    def close(self) -> None:
        for transport in (self.front, self.back):
            if transport is not None:
                transport.close()


class Back(asyncio.DatagramProtocol):
    def __init__(self, relay: Relay) -> None:
        self.relay = relay

    def datagram_received(self, data: bytes, addr: tuple[str, int]) -> None:
        self.relay.answer(data)


async def relay_to(address: str) -> Relay:
    host, port = address.rsplit(":", 1)
    loop = asyncio.get_running_loop()
    relay = Relay((host, int(port)))
    relay.front, _ = await loop.create_datagram_endpoint(
        lambda: relay, local_addr=("127.0.0.1", 0))
    relay.back, _ = await loop.create_datagram_endpoint(
        lambda: Back(relay), local_addr=("127.0.0.1", 0))
    return relay


def client(**keywords) -> BACnetClient:
    defaults = {"interface": "127.0.0.1", "port": 0, "broadcast_address": "127.0.0.1",
                "apdu_timeout_ms": 2000}
    return BACnetClient(**{**defaults, **keywords})


async def plant() -> BACnetServer:
    """A device with an analog input, a commandable analog output, a binary
    value without a Priority_Array, a commandable multi-state value and a
    commandable analog value."""
    server = BACnetServer(DEVICE, "Plant Controller", interface="127.0.0.1", port=0,
                          broadcast_address="127.0.0.1")
    server.add_analog_input(1, "Zone Temp", EngineeringUnits.DEGREES_FAHRENHEIT, 72.5)
    server.add_analog_output(2, "Damper", EngineeringUnits.PERCENT)
    server.add_binary_value(3, "Occupied", present_value_access="writable")
    server.add_multistate_value(4, "Fan Mode", 3)
    server.add_analog_value(5, "Setpoint", profile_name="555-setpoint")
    await server.start()
    local = server.write_property_local
    text = PropertyValue.character_string
    local(AI, PropertyIdentifier.DESCRIPTION, text("North zone"), source_object=None)
    local(BV, PropertyIdentifier.ACTIVE_TEXT, text("In use"), source_object=None)
    local(BV, PropertyIdentifier.INACTIVE_TEXT, text("Empty"), source_object=None)
    for state, name in enumerate(("Off", "Low", "High"), start=1):
        local(MSV, PropertyIdentifier.STATE_TEXT, text(name), array_index=state,
              source_object=None)
    return server


async def many(count: int) -> BACnetServer:
    """A device with ``count`` analog inputs, AI n reading n."""
    server = BACnetServer(DEVICE + 1, "Big Controller", interface="127.0.0.1", port=0,
                          broadcast_address="127.0.0.1")
    for n in range(count):
        server.add_analog_input(n, f"AI {n}", EngineeringUnits.DEGREES_CELSIUS, float(n))
    await server.start()
    return server


async def eventually(check, seconds: float = 5.0) -> None:
    deadline = asyncio.get_running_loop().time() + seconds
    while not check():
        if asyncio.get_running_loop().time() > deadline:
            raise AssertionError("timed out waiting")
        await asyncio.sleep(0.005)


class Lab(unittest.IsolatedAsyncioTestCase):
    """The plant behind a relay."""

    async def asyncSetUp(self) -> None:
        self.server = await plant()
        self.addAsyncCleanup(cleanup, self.server.stop)
        self.relay = await relay_to(await self.server.local_address())
        self.addCleanup(self.relay.close)
        self.address = self.relay.address()


class InventoryTests(Lab):
    async def test_every_object_as_a_record(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            inventory = await dev.inventory()
            support = dev.cov_support()
        # The instance (a handle by address doesn't know it), the summary,
        # the Object_List, then all six objects in one request.
        self.assertEqual(self.relay.services, [READ_PROPERTY, READ_PROPERTY_MULTIPLE,
                                               READ_PROPERTY, READ_PROPERTY_MULTIPLE])
        self.assertIsInstance(inventory, Inventory)
        self.assertEqual(inventory.address, self.address)
        self.assertIsInstance(inventory.summary, DeviceSummary)
        self.assertEqual((inventory.summary.instance, inventory.summary.name),
                         (DEVICE, "Plant Controller"))
        self.assertIsNone(inventory.stopped)
        self.assertEqual(inventory.unread, ())
        # The summary's services say whether the device offers COV.
        self.assertIn(inventory.cov_support, ("offered", "not-offered"))
        self.assertEqual(inventory.cov_support, support)
        self.assertEqual([p.object_id for p in inventory], [OWN, AI, AO, BV, MSV, AV])
        self.assertEqual(len(inventory), 6)
        self.assertIs(inventory[1], inventory.points[1])
        self.assertTrue(all(isinstance(p, Point) for p in inventory.points))
        self.assertIs(inventory.point("ai:1"), inventory.points[1])
        self.assertIs(inventory.point(AV), inventory.points[5])
        self.assertIsNone(inventory.point("ai:99"))

        ai = inventory.point(AI)
        self.assertEqual((ai.address, ai.device_instance), (self.address, DEVICE))
        self.assertEqual((ai.name, ai.description), ("Zone Temp", "North zone"))
        self.assertIs(type(ai.units), EngineeringUnits)
        self.assertEqual(ai.units, EngineeringUnits.DEGREES_FAHRENHEIT)
        self.assertEqual(ai.units_text, "degrees-fahrenheit")
        self.assertFalse(ai.commandable)
        self.assertEqual((ai.state_texts, ai.active_text, ai.inactive_text), ((), None, None))
        self.assertIsNone(ai.number_of_states)
        # Nothing was asked for that the options leave out.
        for field in ("present_value", "status_flags", "event_state", "reliability",
                      "command_priority", "relinquish_default", "cov_increment",
                      "min_present_value", "max_present_value", "polarity", "device_type",
                      "profile_name"):
            self.assertIsNone(getattr(ai, field), field)
        self.assertEqual(ai.problems, ())
        self.assertIsNone(ai.problem(PropertyIdentifier.OBJECT_NAME))
        self.assertIsNone(ai.problem("units"))
        self.assertEqual(repr(ai), "Point(analog-input 1, name='Zone Temp')")

        ao = inventory.point(AO)
        self.assertTrue(ao.commandable)
        self.assertEqual(ao.units, EngineeringUnits.PERCENT)
        self.assertEqual(ao.units_text, "percent")

        bv = inventory.point(BV)
        self.assertEqual((bv.active_text, bv.inactive_text), ("In use", "Empty"))
        self.assertEqual(bv.state_texts, ())
        self.assertIsNone(bv.units)
        self.assertIsNone(bv.units_text)
        # A value object without a Priority_Array: no commands, and the
        # device saying it has none is no problem.
        self.assertFalse(bv.commandable)
        self.assertEqual(bv.problems, ())

        msv = inventory.point(MSV)
        self.assertEqual(msv.state_texts, ("Off", "Low", "High"))
        self.assertEqual(msv.number_of_states, 3)
        self.assertTrue(msv.commandable)

        av = inventory.point(AV)
        self.assertTrue(av.commandable)
        self.assertEqual(inventory.point(OWN).name, "Plant Controller")
        self.assertEqual(repr(inventory), "Inventory(device=2563, points=6)")

    async def test_types_values_settings_and_details(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            await c.write_property(await self.server.local_address(), AO,
                                   PropertyIdentifier.PRESENT_VALUE, PropertyValue.real(40.0),
                                   priority=8)
            inventory = await dev.inventory(
                types=["ai", ObjectType.ANALOG_OUTPUT, "multi-state-value", "av"],
                values=True, settings=True, details=True)
        self.assertEqual([p.object_id for p in inventory], [AI, AO, MSV, AV])
        ai, ao, msv, av = inventory
        self.assertIsInstance(ai.present_value, Value)
        # Units read in the same request name the number.
        self.assertEqual(ai.present_value.text, "72.5 °F")
        self.assertEqual(ai.present_value.value.value, 72.5)
        self.assertIsInstance(ai.status_flags, Value)
        self.assertEqual(ai.event_state.text, "normal")
        self.assertEqual(ao.present_value.text, "40 %")
        self.assertEqual(ao.command_priority.data, 8)
        self.assertIsInstance(ao.relinquish_default, Value)
        self.assertEqual(msv.present_value.text.split(" ")[0], "1")
        self.assertEqual(av.profile_name, "555-setpoint")
        # A Value copies and pickles; the record holding it doesn't.
        self.assertEqual(copy.copy(ai.present_value), ai.present_value)
        self.assertEqual(pickle.loads(pickle.dumps(ai.present_value)), ai.present_value)

    async def test_a_device_refusing_rpm_is_read_property_by_property(self) -> None:
        self.relay.refuse_rpm = True
        async with client() as c:
            dev = await c.device(self.address)
            inventory = await dev.inventory()
        # One refused ReadPropertyMultiple, remembered; ReadProperty after.
        self.assertEqual(self.relay.services.count(READ_PROPERTY_MULTIPLE), 1)
        self.assertGreater(len(self.relay.services), 20)
        self.assertIsNone(inventory.stopped)
        self.assertEqual([p.object_id for p in inventory], [OWN, AI, AO, BV, MSV, AV])
        msv = inventory.point(MSV)
        self.assertEqual((msv.name, msv.state_texts, msv.commandable),
                         ("Fan Mode", ("Off", "Low", "High"), True))
        self.assertEqual(inventory.point(AI).units, EngineeringUnits.DEGREES_FAHRENHEIT)
        self.assertEqual(inventory.point(BV).active_text, "In use")

    async def test_a_record_refuses_copy_and_pickle(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            inventory = await dev.inventory(types=["ai"])
            walk = await dev.points(["ai:1"])
        for result in (inventory, inventory[0], walk):
            with self.subTest(kind=type(result).__name__):
                with self.assertRaisesRegex(TypeError, "cannot pickle"):
                    pickle.dumps(result)
                with self.assertRaises(TypeError):
                    copy.copy(result)
        with self.assertRaises(AttributeError):
            inventory[0].name = "x"  # type: ignore[misc]


class BigDevice(unittest.IsolatedAsyncioTestCase):
    """Sixty analog inputs behind a relay."""

    async def asyncSetUp(self) -> None:
        self.server = await many(60)
        self.addAsyncCleanup(cleanup, self.server.stop)
        self.relay = await relay_to(await self.server.local_address())
        self.addCleanup(self.relay.close)
        self.address = self.relay.address()


class PagingTests(BigDevice):
    async def test_the_records_come_a_batch_at_a_time(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            await dev.resolve_instance()
            walk = await dev.points(types=["ai"])
            self.assertIsInstance(walk, PointIterator)
            # The Object_List alone, read by the await.
            self.assertEqual(self.relay.services, [READ_PROPERTY, READ_PROPERTY])
            self.assertEqual(len(walk), 60)
            self.assertEqual(walk.objects[:2], (ObjectIdentifier(ObjectType.ANALOG_INPUT, 0),
                                                ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)))
            self.assertEqual(repr(walk), "PointIterator(60 objects)")
            first = await anext(walk)
            # One batch of ten, three properties each, for the first point.
            self.assertEqual(self.relay.services[2:], [READ_PROPERTY_MULTIPLE])
            rest = [point async for point in walk]
            # Nothing more is read once the walk is done.
            with self.assertRaises(StopAsyncIteration):
                await anext(walk)
        points = [first, *rest]
        self.assertEqual([p.name for p in points], [f"AI {n}" for n in range(60)])
        self.assertTrue(all(p.units == EngineeringUnits.DEGREES_CELSIUS for p in points))
        self.assertEqual(self.relay.services[2:], [READ_PROPERTY_MULTIPLE] * 6)

    async def test_named_objects_each_once_in_order(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            walk = await dev.points(["ai:3", ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
                                     "ai:3", "analog-input:59"])
            self.assertEqual(self.relay.services, [])
            self.assertEqual([str(o) for o in walk.objects],
                             [str(ObjectIdentifier(ObjectType.ANALOG_INPUT, n))
                              for n in (3, 1, 59)])
            names = [point.name async for point in walk]
            # The handle's instance, then the three objects in one request.
            self.assertEqual(self.relay.services, [READ_PROPERTY, READ_PROPERTY_MULTIPLE])
            # `types` filters a list given too.
            walk = await dev.points(["ai:3", "device:2564"], types=["device"])
            self.assertEqual(len(walk), 1)
            empty = await dev.points([])
            self.assertEqual([p async for p in empty], [])
        self.assertEqual(names, ["AI 3", "AI 1", "AI 59"])

    async def test_breaking_off_stops_the_reads(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            await dev.resolve_instance()
            async for _ in await dev.points():
                break
            sent = len(self.relay.services)
            await asyncio.sleep(0.3)
        self.assertEqual(self.relay.services[2:], [READ_PROPERTY_MULTIPLE])
        self.assertEqual(len(self.relay.services), sent)

    async def test_a_device_refusing_rpm_is_walked_property_by_property(self) -> None:
        self.relay.refuse_rpm = True
        async with client() as c:
            dev = await c.device(self.address)
            await dev.resolve_instance()
            walk = await dev.points(["ai:0", "ai:1", "ai:2"])
            points = [point async for point in walk]
        self.assertEqual([p.name for p in points], ["AI 0", "AI 1", "AI 2"])
        self.assertEqual([p.units_text for p in points], ["degrees-celsius"] * 3)
        services = self.relay.services[1:]
        self.assertEqual(services[0], READ_PROPERTY_MULTIPLE)
        # Three properties of three objects, one request each.
        self.assertEqual(services[1:], [READ_PROPERTY] * 9)


class EarlyEndTests(BigDevice):
    async def test_an_inventory_keeps_what_was_read(self) -> None:
        # The instance, the summary, the Object_List and two batches pass.
        self.relay.passing = 5
        async with client(apdu_timeout_ms=300) as c:
            dev = await c.device(self.address)
            inventory = await dev.inventory(types=["ai"])
        self.assertIsInstance(inventory.stopped, BacnetTimeoutError)
        self.assertEqual([p.name for p in inventory], [f"AI {n}" for n in range(20)])
        self.assertEqual(inventory.unread,
                         tuple(ObjectIdentifier(ObjectType.ANALOG_INPUT, n)
                               for n in range(20, 60)))
        self.assertEqual(repr(inventory), "Inventory(device=2564, points=20, stopped)")

    async def test_a_walk_raises_after_the_records_read(self) -> None:
        # The Object_List, the instance and two batches pass.
        self.relay.passing = 4
        async with client(apdu_timeout_ms=300) as c:
            dev = await c.device(self.address)
            walk = await dev.points()
            seen = []
            with self.assertRaises(BacnetTimeoutError):
                async for point in walk:
                    seen.append(point.name)
            # The Device object and ten inputs, then ten more.
            self.assertEqual(seen, ["Big Controller", *(f"AI {n}" for n in range(20))])
            # Iterating again tries the first object not read again.
            self.relay.forget()
            seen.extend([point.name async for point in walk])
        self.assertEqual(seen, ["Big Controller", *(f"AI {n}" for n in range(60))])

    async def test_a_silent_device_raises_from_the_first_point(self) -> None:
        async with client(apdu_timeout_ms=300) as c:
            dev = await c.device(self.address)
            await dev.resolve_instance()
            walk = await dev.points(types=["ai"])
            self.relay.passing = len(self.relay.services)
            with self.assertRaises(BacnetTimeoutError):
                await anext(walk)
            with self.assertRaises(BacnetTimeoutError):
                await dev.inventory()

    async def test_after_stop_a_walk_raises_runtime_error(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            walk = await dev.points(["ai:0"])
            await c.stop()
            with self.assertRaises(RuntimeError):
                await anext(walk)
            with self.assertRaises(RuntimeError):
                await dev.inventory()
        # Nothing was sent after the stop.
        self.assertEqual(self.relay.services, [])


class CancelTests(BigDevice):
    async def test_cancelling_an_inventory_stops_its_reads(self) -> None:
        self.relay.delay = 0.15
        async with client() as c:
            dev = await c.device(self.address)
            task = asyncio.ensure_future(dev.inventory(types=["ai"]))
            # The instance, the summary, the Object_List, the first batch.
            await eventually(lambda: len(self.relay.services) >= 4)
            task.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await task
            sent = len(self.relay.services)
            await asyncio.sleep(1.0)
            self.assertEqual(len(self.relay.services), sent)
        # Four, or five if a batch's answer beat the cancel; a read the
        # cancel didn't stop sends all nine.
        self.assertLessEqual(sent, 5)

    async def test_a_cancelled_point_loses_nothing(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            await dev.resolve_instance()
            walk = await dev.points(types=["ai"])
            # Answers come late: the first batch is still being read when
            # its step is cancelled.
            self.relay.delay = 0.4
            step = asyncio.ensure_future(anext(walk))
            await eventually(lambda: len(self.relay.services) == 3)
            step.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await step
            # The batch's request went; nothing after it, its late answer
            # included.
            self.assertEqual(self.relay.services[2:], [READ_PROPERTY_MULTIPLE])
            await asyncio.sleep(0.8)
            self.assertEqual(self.relay.services[2:], [READ_PROPERTY_MULTIPLE])
            self.relay.delay = 0.0
            names = [point.name async for point in walk]
        # The cancelled batch is read again, and every point comes once.
        self.assertEqual(names, [f"AI {n}" for n in range(60)])
        self.assertEqual(self.relay.services[2:], [READ_PROPERTY_MULTIPLE] * 7)

    async def test_concurrent_steps_get_consecutive_points(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            walk = await dev.points(types=["ai"])
            # The first batch is read once, by whichever step comes first.
            pair = await asyncio.gather(anext(walk), anext(walk))
            self.assertEqual(sorted(p.name for p in pair), ["AI 0", "AI 1"])
            pair = await asyncio.gather(anext(walk), anext(walk), anext(walk))
            self.assertEqual(sorted(p.name for p in pair), ["AI 2", "AI 3", "AI 4"])
        self.assertEqual(self.relay.services.count(READ_PROPERTY_MULTIPLE), 1)


class StopTests(BigDevice):
    async def test_stop_waits_for_an_inventory_in_flight(self) -> None:
        self.relay.delay = 0.05
        async with client() as c:
            dev = await c.device(self.address)
            task = asyncio.ensure_future(dev.inventory(types=["ai"]))
            await eventually(lambda: len(self.relay.services) >= 2)
            await c.stop()
            # The stop returned after the read: the instance, the summary,
            # the Object_List and all six batches.
            self.assertEqual(len(self.relay.services), 9)
            inventory = await task
            self.assertIsNone(inventory.stopped)
            self.assertEqual(len(inventory), 60)
            with self.assertRaises(RuntimeError):
                await dev.inventory()
            with self.assertRaises(RuntimeError):
                await dev.points()
        self.assertEqual(len(self.relay.services), 9)

    async def test_points_read_before_a_stop_come_out_first(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            await dev.resolve_instance()
            walk = await dev.points(types=["ai"])
            first = await anext(walk)
            await c.stop()
            # The rest of the first batch, already read.
            rest = [await anext(walk) for _ in range(9)]
            with self.assertRaises(RuntimeError):
                await anext(walk)
        self.assertEqual([p.name for p in [first, *rest]], [f"AI {n}" for n in range(10)])
        self.assertEqual(self.relay.services[2:], [READ_PROPERTY_MULTIPLE])


class ArgumentTests(Lab):
    async def test_bad_arguments_raise_when_the_call_is_made(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            for call in (dev.inventory, dev.points):
                with self.subTest(call=call.__name__):
                    for bad in (5, "ai", [5], [None], b"ai"):
                        with self.subTest(types=bad), self.assertRaises(TypeError):
                            call(types=bad)
                    for bad in (["nonsense"], []):
                        with self.subTest(types=bad), self.assertRaises(ValueError):
                            call(types=bad)
                    for keyword in ("values", "settings", "details"):
                        for bad in ("yes", 1, None):
                            with self.subTest(keyword=keyword, value=bad), \
                                    self.assertRaises(TypeError):
                                call(**{keyword: bad})
            for bad in (5, "ai:1", [5], [None]):
                with self.subTest(objects=bad), self.assertRaises(TypeError):
                    dev.points(bad)
            for bad in (["ai:x"], ["nonsense:1"]):
                with self.subTest(objects=bad), self.assertRaises(ValueError):
                    dev.points(bad)
            with self.assertRaises(TypeError):
                dev.inventory(["ai:1"])  # type: ignore[misc]
        self.assertEqual(self.relay.services, [])



class ListTests(BigDevice):
    async def test_a_list_read_element_by_element(self) -> None:
        self.relay.refuse_list = "whole"
        async with client() as c:
            dev = await c.device(self.address)
            walk = await dev.points(types=["ai"])
            # The whole list, refused; its size; then each of 61 elements.
            self.assertEqual(self.relay.services, [READ_PROPERTY] * 63)
            self.assertEqual(len(walk), 60)
            names = [point.name async for point in walk]
        self.assertEqual(names, [f"AI {n}" for n in range(60)])

    async def test_part_of_a_list_raises_naming_the_element(self) -> None:
        self.relay.refuse_list = "whole"
        # The whole list, its size and five elements pass; the device goes
        # silent at the sixth, which raises as a timeout, naming it.
        self.relay.passing = 7
        async with client(apdu_timeout_ms=300) as c:
            dev = await c.device(self.address)
            with self.assertRaisesRegex(BacnetTimeoutError, "element 6 of 61"):
                await dev.points()
        self.assertEqual(len(self.relay.services), 8)

    async def test_part_of_a_list_fails_an_inventory_the_same_way(self) -> None:
        self.relay.refuse_list = "whole"
        # The instance and the summary, then as above.
        self.relay.passing = 9
        async with client(apdu_timeout_ms=300) as c:
            dev = await c.device(self.address)
            with self.assertRaisesRegex(BacnetTimeoutError, "element 6 of 61"):
                await dev.inventory()
        self.assertEqual(len(self.relay.services), 10)

    async def test_a_refused_list_raises_the_refusal(self) -> None:
        self.relay.refuse_list = "all"
        async with client() as c:
            dev = await c.device(self.address)
            with self.assertRaises(BacnetRejectError):
                await dev.points()
            with self.assertRaises(BacnetRejectError):
                await dev.inventory()


class StubParityTests(unittest.TestCase):
    CLASSES = ("Inventory", "Point", "PointIterator")

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

    def test_the_device_calls_match_the_stub(self) -> None:
        from test_relinquish import stub_method

        for name, expected in (
            ("inventory", ["types", "values", "settings", "details"]),
            ("points", ["objects", "types", "values", "settings", "details"]),
        ):
            with self.subTest(method=name):
                runtime = runtime_parameters(getattr(Device, name))
                self.assertEqual([n for n, _ in runtime], expected)
                self.assertEqual(runtime, stub_parameters(stub_method("Device", name)))
                # On the handle only: the client has no second spelling.
                self.assertFalse(hasattr(BACnetClient, name))


if __name__ == "__main__":
    unittest.main()
