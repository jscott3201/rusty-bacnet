"""The ``Device`` handle and discovery by scope (#2561 part 2).

``await client.device(target)`` names a device in the target grammar and
gives a handle whose methods are the high-level calls: ``read``,
``read_many``, ``read_object``, ``write``, ``preview_write``, ``relinquish``,
``priority_array``, ``objects`` and ``summary``. ``discover`` returns the
devices that answered that call, each a target. The tests run servers in
this process; a UDP relay (``test_relinquish``'s) records each confirmed
request a client sends and can go silent part-way.
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
    BACnetRouter,
    BACnetServer,
    BacnetAmbiguousDeviceError,
    BacnetError,
    BacnetLookupError,
    BacnetProtocolError,
    BacnetTimeoutError,
    BacnetWriteDeniedError,
    Device,
    DeviceSummary,
    Discovered,
    ErrorClass,
    ErrorCode,
    ForeignDevice,
    ObjectIdentifier,
    ObjectList,
    ObjectType,
    PriorityView,
    PropertyIdentifier,
    PropertyValue,
    Reading,
    Readings,
    Value,
    WritePolicy,
    WritePreview,
    Written,
)

from test_bip_modes import accepted_registration
from test_relinquish import (
    Lab,
    client,
    close,
    relay_address,
    relay_to,
    runtime_parameters,
    stub_class,
    stub_parameters,
    write_request,
)
from test_router_application import bbmd_port, cleanup

AO = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)
AI = ObjectIdentifier(ObjectType.ANALOG_INPUT, 4)
PV = PropertyIdentifier.PRESENT_VALUE

READ_PROPERTY = 12
READ_PROPERTY_MULTIPLE = 14
WRITE_PROPERTY = 15
DEVICE = 2561


def server(instance: int, **points: float) -> BACnetServer:
    """A device with one analog input, AI 0, and one analog output."""
    made = BACnetServer(
        instance, f"Device {instance}", interface="127.0.0.1", port=0,
        broadcast_address="127.0.0.1",
    )
    made.add_analog_input(0, "Zone Temp", 62, points.get("value", 20.0))
    made.add_analog_output(1, "Damper")
    return made


class ReadTests(Lab):
    async def test_one_property_is_one_read_property(self) -> None:
        async with client() as c:
            await self.command(c, 8, 72.5)
            dev = await c.device(self.address)
            self.assertIsInstance(dev, Device)
            # A local address sends nothing to make its handle.
            self.assertEqual(self.relay.requests, [])
            self.assertEqual((dev.address, dev.instance, dev.follows),
                             (self.address, None, False))
            value = await dev.read("ao:1")
            self.assertEqual(self.relay.services, [READ_PROPERTY])
            self.assertIsInstance(value, Value)
            self.assertEqual((value.text, value.data, value.value.value), ("72.5", 72.5, 72.5))
            self.assertEqual(value.raw, bytes.fromhex("4442910000"))
            # Every way of naming the same property reads the same value.
            for named in (
                lambda: dev.read("ao:1", "pv"),
                lambda: dev.read("ao:1 pv"),
                lambda: dev.read("analog-output:1", "present-value"),
                lambda: dev.read(AO),
                lambda: dev.read(AO, PV),
                lambda: dev.read("ao:1", PV),
            ):
                self.assertEqual(await named(), value)
            # An element, three ways.
            slot = await dev.read("ao:1", "priority-array[8]")
            self.assertEqual(slot.text, "72.5")
            self.assertEqual(await dev.read("ao:1", "priority-array", index=8), slot)
            self.assertEqual(
                await dev.read(AO, PropertyIdentifier.PRIORITY_ARRAY, index=8), slot)
            self.assertEqual(slot.value, await c.read_property(
                self.direct, AO, PropertyIdentifier.PRIORITY_ARRAY, 8))
            name = await dev.read("ao:1", "object-name")
            self.assertEqual(name.value.value, "Damper")

    async def test_bad_arguments_raise_before_the_future(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            for bad in ("nonsense", "ao:1 pv extra", "ao:x"):
                with self.subTest(object=bad), self.assertRaises(ValueError):
                    dev.read(bad)
            with self.assertRaises(ValueError):
                dev.read("ao:1", "no-such-property-name")
            with self.assertRaisesRegex(ValueError, "element 8 and index=9"):
                dev.read("ao:1", "priority-array[8]", index=9)
            for bad in (5, 5.0, None, b"ao:1"):
                with self.subTest(object=bad), self.assertRaises(TypeError):
                    dev.read(bad)
            with self.assertRaises(TypeError):
                dev.read("ao:1", 85)
            with self.assertRaisesRegex(ValueError, "read_object"):
                dev.read_many([("ao:1", "all")])
            with self.assertRaises(ValueError):
                dev.read_many([("ao:1", "pv", "x", "y")])
            # ALL is refused by `read` on await, with nothing sent.
            with self.assertRaisesRegex(ValueError, "read_object"):
                await dev.read("ao:1", "all")
        self.assertEqual(self.relay.requests, [])

    async def test_a_refused_property_raises_the_protocol_error(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            with self.assertRaises(BacnetProtocolError) as caught:
                await dev.read("ai:99")
        self.assertEqual(caught.exception.error_code, ErrorCode.UNKNOWN_OBJECT)

    async def test_read_many_keeps_a_refused_row_as_data(self) -> None:
        async with client() as c:
            await self.command(c, 8, 72.5)
            dev = await c.device(self.address)
            read = await dev.read_many([
                "ao:1 pv", ("ao:1", "units"), ("ai:99", "pv"), ("ao:1", "priority-array", 8),
                AI, ("ao:1", "9999"),  # a property by number, which the device lacks
            ])
        self.assertEqual(self.relay.services, [READ_PROPERTY_MULTIPLE])
        self.assertIsInstance(read, Readings)
        self.assertEqual((len(read), read.via, read.stopped), (6, "rpm", None))
        self.assertTrue(all(isinstance(row, Reading) for row in read))
        self.assertEqual(list(read), list(read.readings))
        self.assertIs(read[0], read.readings[0])
        # Units read in the same call name the number.
        self.assertEqual(read[0].text, "72.5 °C")
        self.assertEqual((read[0].object_id, read[0].property_id, read[0].array_index),
                         (AO, PV, None))
        self.assertTrue(read[0].ok)
        self.assertIsNone(read[0].error)
        # The refused rows are data: nothing was raised, the rest stand.
        missing = read.get("ai:99")
        self.assertIs(missing, read[2])
        self.assertFalse(missing.ok)
        self.assertIsNone(missing.value)
        self.assertIsNone(missing.text)
        self.assertIsInstance(missing.error, BacnetProtocolError)
        self.assertEqual((missing.error.error_class, missing.error.error_code),
                         (ErrorClass.OBJECT, ErrorCode.UNKNOWN_OBJECT))
        unknown = read[5]
        self.assertEqual(unknown.error.error_code, ErrorCode.UNKNOWN_PROPERTY)
        self.assertEqual(read[3].array_index, 8)
        self.assertEqual(read.get("ao:1", "priority-array", index=8).text, "72.5")
        self.assertIsNone(read.get("ao:2"))
        self.assertEqual(repr(read), "Readings(6 readings, via=rpm)")
        self.assertIn("error=BacnetProtocolError", repr(missing))
        self.assertEqual(repr(read[4]), "Reading(analog-input 4 present-value, '0')")

    async def test_read_many_of_nothing_sends_nothing(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            read = await dev.read_many([])
        self.assertEqual((len(read), read.via, read.stopped), (0, None, None))
        self.assertEqual(self.relay.requests, [])

    async def test_an_object_all_or_some(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            some = await dev.read_object("ao:1", ["pv", PropertyIdentifier.UNITS, "object-name"])
            self.assertEqual(self.relay.services, [READ_PROPERTY_MULTIPLE])
            self.assertEqual([row.property_id for row in some],
                             [PV, PropertyIdentifier.UNITS, PropertyIdentifier.OBJECT_NAME])
            self.assertIsNone(some.stopped)
            everything = await dev.read_object(AO)
            self.assertGreater(len(everything), 10)
            self.assertIsNotNone(everything.get(AO, "priority-array"))
            with self.assertRaisesRegex(ValueError, "whole properties"):
                dev.read_object("ao:1", ["priority-array[8]"])

    async def test_objects_and_summary(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            self.assertEqual(dev.cov_support(), "unknown")
            objects = await dev.objects()
            self.assertIsInstance(objects, ObjectList)
            self.assertEqual(len(objects), 5)
            self.assertIn(AO, objects)
            self.assertIn("ai:4", objects)
            self.assertNotIn("ai:99", objects)
            self.assertEqual(objects[0], ObjectIdentifier(ObjectType.DEVICE, DEVICE))
            self.assertEqual(list(objects), list(objects.objects))
            self.assertEqual((objects.indexed, objects.stopped), (False, None))
            self.assertIsNone(dev.instance)
            self.relay.clear()
            summary = await dev.summary()
            # The instance by the wildcard, then the summary in one RPM.
            self.assertEqual(self.relay.services, [READ_PROPERTY, READ_PROPERTY_MULTIPLE])
            self.assertIsInstance(summary, DeviceSummary)
            self.assertEqual((summary.instance, summary.name), (DEVICE, "Plant Controller"))
            self.assertEqual(summary.value("object-name").value.value, "Plant Controller")
            self.assertIsNone(summary.value("description"))
            self.assertIsInstance(summary.readings, Readings)
            # The handle learned its instance and what the device offers.
            self.assertEqual(dev.instance, DEVICE)
            self.assertEqual(dev.cov_support(), "offered")
            self.assertEqual(await dev.resolve_instance(), DEVICE)
            self.assertEqual(repr(dev), f"Device({self.address}, instance={DEVICE}, pinned)")


class WriteTests(Lab):
    async def test_a_write_names_its_priority(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            written = await dev.write("ao:1", "pv", "72.5@8")
            self.assertEqual(self.relay.services, [WRITE_PROPERTY])
            self.assertEqual(write_request(self.relay.requests[0][1]),
                             (85, bytes.fromhex("4442910000"), 8))
            self.assertIsInstance(written, Written)
            self.assertEqual(
                (written.address, written.object_id, written.property_id, written.priority,
                 written.commandable, written.value.text),
                (self.address, AO, PV, 8, "always", "72.5"))
            # The keyword, the same priority twice, a number and a typed value.
            for value, keywords, sent in (
                ("50", {"priority": 9}, 9),
                ("50@9", {"priority": 9}, 9),
                (50.0, {"priority": 9}, 9),
                (50, {"priority": 9}, 9),
                (PropertyValue.real(50.0), {"priority": 9}, 9),
                ("50", {}, 16),  # an output with none asked goes at 16, and says so
            ):
                with self.subTest(value=value, keywords=keywords):
                    self.relay.clear()
                    written = await dev.write(AO, PV, value, **keywords)
                    prop, octets, priority = write_request(self.relay.requests[0][1])
                    self.assertEqual((prop, octets, priority),
                                     (85, bytes.fromhex("4442480000"), sent))
                    self.assertEqual(written.priority, sent)
            # A value object's Present_Value with no priority asked carries none.
            self.relay.clear()
            written = await dev.write("av:3", "pv", "21.5")
            self.assertIsNone(write_request(self.relay.requests[0][1])[2])
            self.assertEqual((written.priority, written.commandable), (None, "optional"))
            self.assertEqual((await dev.read("av:3")).data, 21.5)
            # Text is quoted in the value grammar.
            written = await dev.write("ao:1", "description", '"North damper"')
            self.assertEqual((await dev.read("ao:1", "description")).value.value,
                             "North damper")
            self.assertEqual(written.commandable, "never")

    async def test_what_is_refused_sends_nothing(self) -> None:
        async with client() as c:
            dev = await c.device(self.address)
            # Before the Future exists.
            for bad in (True, None, b"1", [1]):
                with self.subTest(value=bad), self.assertRaises(TypeError):
                    dev.write("ao:1", "pv", bad)
            for bad in (0, 17):
                with self.subTest(priority=bad), self.assertRaises(ValueError):
                    dev.write("ao:1", "pv", "1", priority=bad)
            with self.assertRaises(TypeError):
                dev.write("ao:1", "pv", "1", priority="8")
            with self.assertRaises(TypeError):
                dev.write("ao:1", "pv")  # type: ignore[call-arg]
            # On await.
            with self.assertRaisesRegex(ValueError, "priority 8 and the priority argument 9"):
                await dev.write("ao:1", "pv", "5@8", priority=9)
            with self.assertRaises(ValueError):
                await dev.write("ao:1", "pv", "not a number")
            with self.assertRaisesRegex(ValueError, "relinquish"):
                await dev.write("ao:1", "pv", "null", priority=8)
            with self.assertRaisesRegex(ValueError, "read-only"):
                await dev.write("ao:1", "priority-array[8]", "1")
            with self.assertRaisesRegex(ValueError, "isn't commandable"):
                await dev.write("ao:1", "description", '"x"', priority=8)
            for guarded in WritePolicy.GUARDED:
                with self.subTest(priority=guarded):
                    with self.assertRaises(BacnetWriteDeniedError) as caught:
                        await dev.write("ao:1", "pv", f"1@{guarded}")
                    self.assertEqual((caught.exception.priority, caught.exception.relinquish),
                                     (guarded, False))
                    with self.assertRaises(BacnetWriteDeniedError):
                        await dev.relinquish("ao:1", guarded)
        self.assertEqual(self.relay.requests, [])

    async def test_a_policy_that_allows_a_priority(self) -> None:
        policy = WritePolicy().allow_priorities([5])
        async with client(write_policy=policy) as c:
            dev = await c.device(self.address)
            self.assertEqual((await dev.write("ao:1", "pv", "1@5")).priority, 5)
            with self.assertRaises(BacnetWriteDeniedError):
                await dev.write("ao:1", "pv", "1@6")

    async def test_a_preview_reads_and_sends_no_write(self) -> None:
        async with client() as c:
            await self.command(c, 8, 72.5)
            dev = await c.device(self.address)
            preview = await dev.preview_write("ao:1", "pv", "50", priority=9)
            self.assertEqual(self.relay.services, [READ_PROPERTY_MULTIPLE])
            self.assertIsInstance(preview, WritePreview)
            self.assertEqual(
                (preview.address, preview.object_id, preview.property_id, preview.array_index,
                 preview.value.text, preview.priority, preview.occupancy, preview.before),
                (self.address, AO, PV, None, "50", 9, "empty", None))
            # Priority 8 keeps control.
            self.assertEqual((preview.effect, preview.held_by), ("unchanged", 8))
            self.assertIsNone(preview.changes_from)
            self.assertIsInstance(preview.priorities, PriorityView)
            self.assertEqual(preview.priorities.in_control, 8)
            self.assertIsNone(preview.denied)
            taking = await dev.preview_write("ao:1", "pv", "50", priority=8)
            self.assertEqual(
                (taking.occupancy, taking.before.text, taking.effect, taking.changes_from,
                 taking.changes_to, taking.held_by),
                ("held", "72.5", "changes", "72.5", "50", None))
            # A guarded priority is reported, not raised.
            guarded = await dev.preview_write("ao:1", "pv", "50@1")
            self.assertIsInstance(guarded.denied, BacnetWriteDeniedError)
            self.assertEqual(guarded.denied.priority, 1)
            plain = await dev.preview_write("ao:1", "description", '"x"')
            self.assertEqual((plain.occupancy, plain.effect, plain.priority),
                             ("not-commandable", "not-present-value", None))
            self.assertNotIn(WRITE_PROPERTY, self.relay.services)
            self.assertEqual((await dev.read("ao:1")).text, "72.5")

    async def test_the_priority_calls_by_text(self) -> None:
        async with client() as c:
            await self.command(c, 8, 72.5)
            dev = await c.device(self.address)
            view = await dev.priority_array("ao:1")
            self.assertEqual((view.object_id, view.in_control), (AO, 8))
            self.relay.clear()
            after = await dev.relinquish("ao:1", priority=8)
            self.assertEqual(self.relay.services, [WRITE_PROPERTY, READ_PROPERTY_MULTIPLE])
            self.assertEqual(write_request(self.relay.requests[0][1]), (85, b"\x00", 8))
            self.assertIsNone(after.in_control)
            with self.assertRaises(TypeError):
                dev.relinquish("ao:1")  # type: ignore[call-arg]
            with self.assertRaisesRegex(ValueError, "Priority_Array"):
                await dev.priority_array("ai:4")


class SilentPartWayTests(unittest.IsolatedAsyncioTestCase):
    async def test_read_many_keeps_what_was_read(self) -> None:
        # Twenty objects are two batches: the first passes, the second
        # goes unanswered.
        made = BACnetServer(2562, interface="127.0.0.1", port=0,
                            broadcast_address="127.0.0.1")
        for n in range(20):
            made.add_analog_input(n, f"AI {n}", 62, float(n))
        await made.start()
        self.addAsyncCleanup(cleanup, made.stop)
        relay = await relay_to(await made.local_address(), passing=1)
        self.addCleanup(close, relay)
        async with client(apdu_timeout_ms=300) as c:
            dev = await c.device(relay_address(relay))
            read = await dev.read_many([f"ai:{n}" for n in range(20)])
            self.assertEqual(relay.services, [READ_PROPERTY_MULTIPLE] * 2)
            # A single read of a silent device raises the same error, and
            # an object's read keeps nothing.
            with self.assertRaises(BacnetTimeoutError):
                await dev.read("ai:0")
            with self.assertRaises(BacnetTimeoutError):
                await dev.read_object("ai:0")
        self.assertEqual([row.text for row in read], [str(n) for n in range(16)])
        self.assertIsInstance(read.stopped, BacnetTimeoutError)
        self.assertEqual(repr(read), "Readings(16 readings, via=rpm, stopped)")


class TargetTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.first = server(3001, value=21.0)
        self.second = server(3002, value=22.0)
        for made in (self.first, self.second):
            await made.start()
            self.addAsyncCleanup(cleanup, made.stop)
        self.a = await self.first.local_address()
        self.b = await self.second.local_address()

    async def test_discover_returns_this_calls_answers_only(self) -> None:
        async with client() as c:
            found = await c.discover(scope=self.a, wait=0.3)
            self.assertEqual([(row.address, row.instance, row.network, row.duplicate)
                              for row in found], [(self.a, 3001, 0, False)])
            self.assertIsInstance(found[0], Discovered)
            self.assertEqual(found[0].entry.vendor_id, 555)
            self.assertEqual(repr(found[0]), f"Discovered({self.a}, instance=3001)")
            # The table holds the first device; only the second answers now.
            again = await c.discover(scope=self.b, wait=0.3)
            self.assertEqual([row.instance for row in again], [3002])
            self.assertEqual(len(await c.discovered_devices()), 2)
            # A range the device is outside of, and a local broadcast no
            # server here hears: nothing answered, so nothing is listed.
            self.assertEqual(await c.discover(scope=self.a, low=1, high=2, wait=0.2), [])
            self.assertEqual(await c.discover(wait=0.2), [])
            # A row is a target, pinned, knowing its instance.
            dev = await c.device(found[0])
            self.assertEqual((dev.address, dev.instance, dev.follows), (self.a, 3001, False))
            self.assertEqual((await dev.read("ai:0")).data, 21.0)
            row = (await c.discovered_devices())[0]
            pinned = await c.device(row)
            self.assertEqual(pinned.instance, row.object_identifier.instance)

    async def test_an_i_am_from_outside_the_scope_is_not_listed(self) -> None:
        async with client() as c:
            # Two directed discoveries at once: each lists its own station,
            # though the client hears both I-Ams during both waits.
            first, second = await asyncio.gather(
                c.discover(scope=self.a, wait=0.5), c.discover(scope=self.b, wait=0.5))
            self.assertEqual([row.instance for row in first], [3001])
            self.assertEqual([row.instance for row in second], [3002])
            # A remote network's discovery lists no local device that
            # answers another Who-Is meanwhile, and a local one lists it.
            remote, local, _ = await asyncio.gather(
                c.discover(scope=7, wait=0.5), c.discover(wait=0.5),
                c.who_is(scope=self.a))
            self.assertEqual(remote, [])
            self.assertEqual([row.instance for row in local], [3001])
            self.assertEqual(len(await c.discovered_devices()), 2)

    async def test_discover_arguments(self) -> None:
        async with client() as c:
            for keywords in ({"low": 5}, {"high": 5}, {"low": 9, "high": 1},
                             {"high": 4194304, "low": 0}, {"wait": -1}, {"wait": 3601},
                             {"wait": float("nan")}, {"scope": "everywhere"}, {"scope": 0},
                             {"scope": 65535}, {"scope": "1234"}, {"scope": "2:1234"},
                             {"scope": "2:0x05"}):
                with self.subTest(keywords=keywords), self.assertRaises(ValueError):
                    c.discover(**keywords)
            for scope in (True, 1.5, b"local"):
                with self.subTest(scope=scope), self.assertRaises(TypeError):
                    c.discover(scope=scope)
            with self.assertRaises(TypeError):
                c.discover(timeout_ms=100)  # type: ignore[call-arg]
            with self.assertRaises(TypeError):
                c.discover("global")  # type: ignore[misc]  # keywords only
            for bad in (-1, 3601, float("nan")):
                with self.subTest(wait=bad), self.assertRaisesRegex(ValueError, "0 to 3600"):
                    c.discover_routers(wait=bad)
                with self.subTest(wait=bad), self.assertRaisesRegex(ValueError, "0 to 3600"):
                    c.who_has_replies("x", wait=bad)
            self.assertFalse(hasattr(c, "who_is_directed"))
            # A station the link can't reach is refused on await.
            with self.assertRaisesRegex(ValueError, "MS/TP"):
                await c.discover(scope="mstp:5", wait=0)
            # who_is takes the same directed scope, where who_is_directed was.
            await c.who_is(scope=self.a)
            for _ in range(100):
                if await c.discovered_devices():
                    break
                await asyncio.sleep(0.02)
            self.assertEqual([d.object_identifier.instance
                              for d in await c.discovered_devices()], [3001])

    async def test_an_instance_follows_and_an_unknown_one_is_a_lookup_error(self) -> None:
        async with client() as c:
            await c.discover(scope=self.a, wait=0.3)
            for target in (3001, "3001"):
                dev = await c.device(target)
                self.assertEqual((dev.address, dev.instance, dev.follows), (self.a, 3001, True))
                self.assertEqual((await dev.read("ai:0")).data, 21.0)
                self.assertEqual(repr(dev), f"Device({self.a}, instance=3001, follows)")
            for bad in (-1, 4194303, 2**40):
                with self.subTest(target=bad), self.assertRaises(ValueError):
                    c.device(bad)
            for bad in (True, 1.5, None, b"1", (0, "x"), (1, 2, 3)):
                with self.subTest(target=bad), self.assertRaises(TypeError):
                    c.device(bad)
            with self.assertRaises(ValueError):
                c.device("not a target")
            # The wildcard instance and past it, as text too, at once.
            for bad in ("4194303", "4194304", "2:4194303"):
                with self.subTest(target=bad), self.assertRaisesRegex(ValueError, "4194302"):
                    c.device(bad)
            self.assertEqual((await c.device("3001")).instance, 3001)
        # No address claims 3999, and no device hears the one Who-Is for it.
        async with client(apdu_timeout_ms=300) as c:
            with self.assertRaises(BacnetLookupError) as caught:
                await c.device(3999)
        error = caught.exception
        self.assertIsInstance(error, LookupError)
        self.assertIsInstance(error, BacnetError)
        self.assertNotIsInstance(error, BacnetAmbiguousDeviceError)
        # The instance named, though no address claims it.
        self.assertEqual((error.target, error.instance, error.candidates), ("3999", 3999, []))
        self.assertIn("resolve_scope", str(error))
        self.assertNotIn("with_resolve_scope", str(error))

    async def test_two_addresses_claiming_an_instance(self) -> None:
        twin = server(3001, value=99.0)
        await twin.start()
        self.addAsyncCleanup(cleanup, twin.stop)
        other = await twin.local_address()
        async with client() as c:
            await c.discover(scope=self.a, wait=0.3)
            [row] = await c.discover(scope=other, wait=0.3)
            self.assertTrue(row.duplicate)
            self.assertEqual(repr(row), f"Discovered({other}, instance=3001, duplicate)")
            with self.assertRaises(BacnetAmbiguousDeviceError) as caught:
                await c.device(3001)
            error = caught.exception
            self.assertIsInstance(error, BacnetLookupError)
            self.assertIsInstance(error, LookupError)
            self.assertEqual(error.instance, 3001)
            self.assertEqual(len(error.candidates), 2)
            # The typed calls raise the same class for the same instance.
            with self.assertRaises(BacnetAmbiguousDeviceError):
                await c.read_property_from_device(3001, ObjectIdentifier(
                    ObjectType.ANALOG_INPUT, 0), PV)
            # Each candidate is a target, and so is each address.
            values = set()
            for candidate in error.candidates:
                dev = await c.device(candidate)
                self.assertFalse(dev.follows)
                values.add((await dev.read("ai:0")).data)
            self.assertEqual(values, {21.0, 99.0})
            self.assertEqual((await (await c.device(other)).read("ai:0")).data, 99.0)
            self.assertEqual((await (await c.device(row)).read("ai:0")).data, 99.0)

    async def test_a_following_handle_moves_with_its_device(self) -> None:
        # The device answers at one address, then only at another: the same
        # instance, heard there, while the first has gone silent.
        moved = server(3001, value=55.0)
        await moved.start()
        self.addAsyncCleanup(cleanup, moved.stop)
        new = await moved.local_address()
        relay = await relay_to(self.a)
        self.addCleanup(close, relay)
        old = relay_address(relay)
        async with client(apdu_timeout_ms=300) as c:
            await c.discover(scope=old, wait=0.3)
            following = await c.device(3001)
            pinned = await c.device(old)
            self.assertEqual((following.address, pinned.address), (old, old))
            self.assertEqual((await following.read("ai:0")).data, 21.0)
            self.assertEqual((await pinned.summary()).instance, 3001)
            self.assertIsNone(await pinned.moved_to())
            # The old address goes silent, the table forgets it, and the
            # device is heard at the new one.
            relay.passing = 0
            await c.clear_devices()
            await c.discover(scope=new, wait=0.3)
            # One call: a timeout at the old address, then the answer from
            # the new one.
            self.assertEqual((await following.read("ai:0")).data, 55.0)
            self.assertEqual(following.address, new)
            # A pinned handle stays, and says where its instance is now.
            self.assertEqual(pinned.address, old)
            self.assertEqual(await pinned.moved_to(), new)
            with self.assertRaises(BacnetTimeoutError):
                await pinned.read("ai:0")


class LifecycleTests(Lab):
    async def test_a_handle_after_stop_and_after_a_restart(self) -> None:
        c = client()
        async with c:
            await c.discover(scope=self.address, wait=0.3)
            pinned = await c.device(self.address)
            following = await c.device(DEVICE)
            self.assertEqual((await pinned.read("ai:4")).text, "0")
            self.assertEqual((await following.read("ai:4")).text, "0")
        # The stop above succeeded with both handles alive. A call now
        # raises what any call on a stopped client raises, and the getters
        # say what was last known.
        for dev in (pinned, following):
            with self.assertRaisesRegex(RuntimeError, "not started"):
                await dev.read("ai:4")
            with self.assertRaisesRegex(RuntimeError, "not started"):
                await dev.write("ao:1", "pv", "1")
            self.assertEqual(dev.address, self.address)
            self.assertEqual(dev.cov_support(), "unknown")
        self.assertEqual((following.instance, following.follows), (DEVICE, True))
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await c.device(self.address)
        # Started again: a pinned handle works as before. A following one
        # looks its instance up afresh, which the new discovery table
        # doesn't hold until the device is heard again.
        async with c:
            self.assertEqual((await pinned.read("ai:4")).text, "0")
            with self.assertRaises(BacnetLookupError):
                await following.read("ai:4")
            await c.discover(scope=self.address, wait=0.3)
            self.assertEqual((await following.read("ai:4")).text, "0")
            self.assertEqual(following.address, self.address)

    async def test_a_dropped_handle_is_forgotten(self) -> None:
        async with client() as c:
            for _ in range(50):
                dev = await c.device(self.address)
                await dev.read("ai:4")
                del dev
        # The client stopped cleanly however many handles came and went.


class InFlightTests(Lab):
    passing = 0

    async def test_stop_waits_for_a_call_in_flight(self) -> None:
        c = client(apdu_timeout_ms=200)
        await c.__aenter__()
        dev = await c.device(self.address)
        pending = asyncio.ensure_future(dev.read("ai:4"))
        async with asyncio.timeout(10):
            while not self.relay.requests:
                await asyncio.sleep(0.01)
        # The stop waits for the call, which ends when its request times
        # out, and then stops: the handle holds nothing back.
        await asyncio.wait_for(c.stop(), 10)
        with self.assertRaises(BacnetTimeoutError):
            await pending
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await dev.read("ai:4")

    async def test_stop_after_a_call_cancelled_mid_flight(self) -> None:
        c = client(apdu_timeout_ms=10_000)
        await c.__aenter__()
        try:
            dev = await c.device(self.address)
            pending = asyncio.ensure_future(dev.read_many(["ai:4", "ao:1"]))
            async with asyncio.timeout(10):
                while not self.relay.requests:
                    await asyncio.sleep(0.01)
            pending.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await pending
            # A getter between the cancel and the stop keeps no handle.
            self.assertEqual(dev.address, self.address)
        finally:
            # Long before the request would time out.
            await asyncio.wait_for(c.stop(), 5)
        # The relay passes everything from here on, whatever it did with
        # an invoke ID before.
        self.relay.passing = 1_000_000
        self.relay.seen.clear()
        async with c:
            self.assertEqual((await dev.read("ai:4")).text, "0")


class RoutedTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.router = BACnetRouter([bbmd_port(1), bbmd_port(5)], application="client")
        await self.router.start()
        self.addAsyncCleanup(cleanup, self.router.close)
        self.ports = await self.router.ports()
        self.locals = {}
        for network, port in zip((1, 5), self.ports):
            made = BACnetServer(7000 + network, interface="127.0.0.1", port=0,
                                foreign_device=ForeignDevice(port.address, ttl=60))
            made.add_analog_input(0, f"Sensor {network}", 62, 20.5 + network)
            made.add_analog_output(1, "Damper")
            await made.start()
            self.addAsyncCleanup(cleanup, made.stop)
            await accepted_registration(made)
            self.locals[network] = await made.local_address()

    async def test_a_link_client_reaches_a_routed_device(self) -> None:
        # A client on network 1 (a foreign device of the router's port).
        c = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000,
                         foreign_device=ForeignDevice(self.ports[0].address, ttl=60))
        async with c:
            # A routed instance asks its network, then follows.
            dev = await asyncio.wait_for(c.device("5:7005"), 10)
            self.assertEqual((dev.instance, dev.follows), (7005, True))
            self.assertTrue(dev.address.startswith("5:0x"), dev.address)
            self.assertEqual((await dev.read("ai:0")).data, 25.5)
            written = await dev.write("ao:1", "pv", "33@9")
            self.assertEqual((written.address, written.priority), (dev.address, 9))
            self.assertEqual((await dev.relinquish("ao:1", 9)).in_control, None)
            self.assertEqual((await dev.summary()).instance, 7005)
            self.assertEqual(len(await dev.read_many(["ai:0", "ao:1"])), 2)
            # Its address is a target too, pinned.
            again = await c.device(dev.address)
            self.assertEqual((again.address, again.instance, again.follows),
                             (dev.address, 7005, False))
            self.assertEqual((await again.read("ai:0")).data, 25.5)
            # Discovery by network lists the routed device, by its network.
            found = await c.discover(scope=5, wait=1.0)
            self.assertEqual([(row.instance, row.network, row.address) for row in found],
                             [(7005, 5, dev.address)])
            self.assertEqual((await (await c.device(found[0])).read("ai:0")).data, 25.5)
            # A network with no router known is a lookup error.
            with self.assertRaises(BacnetLookupError):
                await asyncio.wait_for(c.device("9:0x05"), 10)

    async def test_a_router_client_names_devices_by_network(self) -> None:
        c = BACnetClient(transport="router", router=self.router, apdu_timeout_ms=2000)
        async with c:
            self.assertIsNone(c.resolve_scope)
            # Works: a routed instance, a routed address, and an instance
            # the client has heard from.
            for network in (1, 5):
                dev = await asyncio.wait_for(c.device(f"{network}:{7000 + network}"), 10)
                self.assertEqual((await dev.read("ai:0")).data, 20.5 + network)
                pinned = await c.device(dev.address)
                self.assertEqual((await pinned.priority_array("ao:1")).in_control, None)
            known = await c.device(7005)
            self.assertEqual((await known.read("ai:0")).data, 25.5)
            self.assertEqual(sorted(row.instance for row in
                                    await c.discover(scope="global", wait=1.0)), [7001, 7005])
            # Raises, with nothing sent: a local address, a bare instance
            # it hasn't heard from (it has no network to ask on), and a
            # local or directed discovery.
            local = self.locals[1]
            for target in (local, (0, bytes([127, 0, 0, 1, 0xBA, 0xC0]))):
                with self.subTest(target=target):
                    with self.assertRaisesRegex(ValueError, "no network of its own"):
                        await c.device(target)
            with self.assertRaisesRegex(BacnetLookupError, "no network of its own"):
                await c.device(7999)
            for scope in (None, "local", local):
                with self.subTest(scope=scope):
                    with self.assertRaisesRegex(ValueError, "no network of its own"):
                        c.discover(scope=scope)
        with self.assertRaisesRegex(ValueError, "resolve_scope='local'"):
            BACnetClient(transport="router", router=self.router, resolve_scope="local")

    async def test_a_router_client_given_a_scope_asks_there(self) -> None:
        # Given a network to ask on, a bare instance is looked for there.
        asking = BACnetClient(transport="router", router=self.router, apdu_timeout_ms=2000,
                              resolve_scope=5)
        self.assertEqual(asking.resolve_scope, 5)
        async with asking:
            dev = await asyncio.wait_for(asking.device(7005), 10)
            self.assertEqual((dev.instance, dev.follows), (7005, True))
            self.assertEqual((await dev.read("ai:0")).data, 25.5)
            with self.assertRaises(BacnetLookupError):
                await asyncio.wait_for(asking.device(7001), 10)  # it is on network 1


class ResolveScopeTests(unittest.TestCase):
    def test_the_keyword(self) -> None:
        self.assertEqual(client().resolve_scope, "local")
        self.assertEqual(client(resolve_scope="local").resolve_scope, "local")
        self.assertEqual(client(resolve_scope="global").resolve_scope, "global")
        self.assertEqual(client(resolve_scope=7).resolve_scope, 7)
        for bad in ("site", 0, 65535, -1):
            with self.subTest(scope=bad), self.assertRaises(ValueError):
                client(resolve_scope=bad)
        for bad in (True, 1.5, b"local"):
            with self.subTest(scope=bad), self.assertRaises(TypeError):
                client(resolve_scope=bad)


class CopyPickleTests(Lab):
    async def test_a_value_copies_and_the_results_do_not(self) -> None:
        async with client() as c:
            await self.command(c, 8, 72.5)
            dev = await c.device(self.address)
            read = await dev.read_many([("ao:1", "pv"), ("ao:1", "units"), ("ai:99", "pv")])
            written = await dev.write("ao:1", "pv", "50@9")
            preview = await dev.preview_write("ao:1", "pv", "50@9")
            view = await dev.priority_array("ao:1")
            objects = await dev.objects()
            summary = await dev.summary()
            [found] = await c.discover(scope=self.direct, wait=0.3)
            value = read[0].value
            self.assertEqual(value.text, "72.5 °C")
            copies = [copy.copy(value), copy.deepcopy(value)] + [
                pickle.loads(pickle.dumps(value, protocol))
                for protocol in range(pickle.HIGHEST_PROTOCOL + 1)
            ]
            for copied in copies:
                self.assertIs(type(copied), Value)
                self.assertEqual(copied, value)
                self.assertEqual(hash(copied), hash(value))
                self.assertEqual(
                    (copied.text, copied.data, copied.raw, copied.value, repr(copied)),
                    (value.text, value.data, value.raw, value.value, repr(value)))
            # Equal by what they hold (octets, text and data), whichever
            # call read them and whatever they are a value of: the slot at
            # priority 8 and Present_Value hold the same REAL.
            self.assertEqual(await dev.read("ao:1", "units"), read[1].value)
            slot, present = await dev.read("ao:1", "priority-array[8]"), await dev.read("ao:1")
            self.assertEqual((slot.raw, slot.text), (present.raw, present.text))
            self.assertEqual(slot, present)
            self.assertEqual(hash(slot), hash(present))
            # A pickle still knows which it was.
            self.assertNotEqual(slot.__reduce__()[1], present.__reduce__()[1])
            self.assertNotEqual(value, read[1].value)
            self.assertNotEqual(value, present)  # "72.5 °C" is other text
            self.assertNotEqual(value, 72.5)
            self.assertIs(value.__eq__(72.5), NotImplemented)
            self.assertEqual(len({value, copies[0], read[1].value}), 2)
            for result in (read, read[0], read[2], written, preview, view, objects, summary,
                           found, dev):
                with self.subTest(kind=type(result).__name__):
                    with self.assertRaisesRegex(TypeError, "cannot pickle"):
                        pickle.dumps(result)
                    with self.assertRaises(TypeError):
                        copy.copy(result)
            with self.assertRaises(AttributeError):
                read[0].value = None  # type: ignore[misc]


class StubParityTests(unittest.TestCase):
    CLASSES = ("Device", "Discovered", "Reading", "Readings", "ObjectList", "DeviceSummary",
               "WritePreview", "Written", "Value")

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

    def test_the_client_calls_match_the_stub(self) -> None:
        from test_relinquish import stub_method

        for name, expected in (
            ("device", ["target"]),
            ("discover", ["scope", "low", "high", "wait"]),
        ):
            with self.subTest(method=name):
                runtime = runtime_parameters(getattr(BACnetClient, name))
                self.assertEqual([n for n, _ in runtime], expected)
                self.assertEqual(runtime, stub_parameters(stub_method("BACnetClient", name)))
        client_stub = {node.name for node in stub_class("BACnetClient").body
                       if isinstance(node, ast.FunctionDef)}
        self.assertNotIn("who_is_directed", client_stub)
        self.assertIn("resolve_scope", client_stub)
        self.assertEqual(runtime_parameters(BACnetClient)[-1],
                         ("resolve_scope", inspect.Parameter.KEYWORD_ONLY))

    def test_the_exception_tree_matches_the_stub(self) -> None:
        for name, bases in (
            ("BacnetLookupError", ["BacnetError", "LookupError"]),
            ("BacnetAmbiguousDeviceError", ["BacnetLookupError"]),
            ("BacnetPagingError", ["BacnetError"]),
        ):
            with self.subTest(exception=name):
                node = stub_class(name)
                self.assertEqual([base.id for base in node.bases], bases)  # type: ignore[attr-defined]
                runtime = getattr(rusty_bacnet, name)
                self.assertEqual([base.__name__ for base in runtime.__bases__], bases)
        for attribute in ("target", "instance", "candidates"):
            self.assertIsNone(getattr(BacnetLookupError, attribute))
        self.assertIsNone(rusty_bacnet.BacnetPagingError.service)


if __name__ == "__main__":
    unittest.main()
