"""Every value class copies and pickles (#1500).

ObjectIdentifier, PropertyValue and BACnetTimeStamp belong to the
rusty_bacnet module, as every class it exports does, and copy.copy,
copy.deepcopy and pickle protocols 0 to 5 give back an equal value of the
same class. An ObjectIdentifier rebuilds through its constructor, a
PropertyValue through the constructor its tag names (a typed constructed
element from the octets it was read from), and a BACnetTimeStamp from its
CHOICE's octets, so a timestamp a peer sent with a field outside the ranges
the constructors check copies too.
"""

from __future__ import annotations

import asyncio
import copy
import pickle
import socket
import unittest
from typing import Any

import rusty_bacnet
from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BACnetTimeStamp,
    BitString,
    Multicast,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

P = PropertyIdentifier
AI_1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
SCHED_1 = ObjectIdentifier(ObjectType.SCHEDULE, 1)
CAL_1 = ObjectIdentifier(ObjectType.CALENDAR, 1)
ACC_1 = ObjectIdentifier(ObjectType.ACCUMULATOR, 1)
RIGHTS_1 = ObjectIdentifier(ObjectType.ACCESS_RIGHTS, 1)
REMOTE_USER = (ObjectIdentifier(ObjectType.DEVICE, 99),
               ObjectIdentifier(ObjectType.ACCESS_USER, 5))


def value_classes() -> dict[str, type]:
    """Every class rusty_bacnet exports that compares by value, the enums
    aside (test_enum_copy_pickle covers them). Found at runtime, so a new
    one fails the test below until it has samples."""
    return {
        name: cls
        for name, cls in vars(rusty_bacnet).items()
        if isinstance(cls, type)
        and not issubclass(cls, BaseException)
        and "__eq__" in vars(cls)
        and not callable(getattr(cls, "from_raw", None))
    }


def samples() -> dict[type, list[Any]]:
    """Values of each class, edges of each field included."""
    oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
    one = [PropertyValue.real(1.0), PropertyValue.double(1.0),
           PropertyValue.unsigned(1), PropertyValue.enumerated(1)]
    return {
        ObjectIdentifier: [
            oid,
            ObjectIdentifier(ObjectType.DEVICE, 4194303),  # the wildcard instance
            ObjectIdentifier(ObjectType.from_raw(1023), 0),  # a proprietary type
        ],
        PropertyValue: [
            PropertyValue.null(),
            PropertyValue.boolean(False),
            PropertyValue.boolean(True),
            PropertyValue.unsigned(2**64 - 1),
            PropertyValue.signed(-(2**31)),
            PropertyValue.real(1.1),  # not exactly a float
            PropertyValue.double(1e300),
            PropertyValue.character_string(""),
            PropertyValue.character_string("Lobby – zone 3 ✓"),
            PropertyValue.octet_string(b"\x00\xff"),
            PropertyValue.enumerated(2**32 - 1),
            PropertyValue.object_identifier(oid),
            PropertyValue.date(2026, 3, 21, 6),
            PropertyValue.date(1900, 14, 34, 7),
            PropertyValue.date(2154, 1, 1, 1),
            PropertyValue.date(255, 255, 255, 255),
            PropertyValue.time(23, 59, 59, 99),
            PropertyValue.time(255, 255, 255, 255),
            PropertyValue.bit_string(3, b"\xa0"),
            PropertyValue.application_data(b"\x0e\x21\x01\x0f"),
            PropertyValue.list([]),
            # Equal as Python values, unequal as BACnet ones: the copy keeps
            # each item's tag.
            PropertyValue.list([*one, PropertyValue.list(one)]),
        ],
        BitString: [
            BitString(0, b""),
            BitString(3, b"\xa0"),
            BitString(4, b"\x40", PropertyIdentifier.STATUS_FLAGS),
        ],
        Multicast: [
            Multicast("239.255.66.1"),
            Multicast("224.0.1.0", ttl=255, port=47809),
        ],
        BACnetTimeStamp: [
            BACnetTimeStamp.sequence_number(0),
            BACnetTimeStamp.sequence_number(65_535),
            BACnetTimeStamp.time(23, 59, 59, 99),
            BACnetTimeStamp.time(255, 255, 255, 255),
            BACnetTimeStamp.date_time((2026, 12, 25, 5), (8, 0, 0, 0)),
            BACnetTimeStamp.date_time((1900, 14, 34, 7), (0, 0, 0, 0)),
            BACnetTimeStamp.date_time((255, 255, 255, 255), (255, 255, 255, 255)),
        ],
    }


def copies(value: Any) -> list[Any]:
    """`value` through copy.copy, copy.deepcopy and every pickle protocol."""
    return [copy.copy(value), copy.deepcopy(value)] + [
        pickle.loads(pickle.dumps(value, protocol))
        for protocol in range(pickle.HIGHEST_PROTOCOL + 1)
    ]


class ValueCopyPickleTests(unittest.TestCase):
    def assert_copies(self, value: Any) -> None:
        cls = type(value)
        for copied in copies(value):
            self.assertIs(type(copied), cls)
            self.assertEqual(copied, value)
            self.assertEqual(repr(copied), repr(value))
            if cls.__hash__ is not None:
                self.assertEqual(hash(copied), hash(value))
            for accessor in ("tag", "kind", "value", "object_type", "instance"):
                if hasattr(value, accessor):
                    self.assertEqual(getattr(copied, accessor), getattr(value, accessor))

    def test_every_value_class_copies_and_pickles(self) -> None:
        classes = value_classes()
        cases = samples()
        self.assertEqual(set(classes.values()), set(cases))
        for name, cls in classes.items():
            with self.subTest(cls=name):
                self.assertEqual(cls.__module__, "rusty_bacnet")
                for value in cases[cls]:
                    with self.subTest(value=repr(value)):
                        self.assert_copies(value)

    def test_every_exported_class_belongs_to_the_module(self) -> None:
        classes = {name: cls for name, cls in vars(rusty_bacnet).items()
                   if isinstance(cls, type)}
        self.assertIn("BACnetServer", classes)
        for name, cls in classes.items():
            with self.subTest(cls=name):
                self.assertEqual(cls.__module__, "rusty_bacnet")
                self.assertEqual(cls.__qualname__, name)

    def test_a_pickle_names_the_constructor_and_full_year(self) -> None:
        dumped = pickle.dumps(PropertyValue.real(1.5))
        for name in (b"rusty_bacnet", b"PropertyValue", b"real"):
            self.assertIn(name, dumped)
        self.assertIn(b"ObjectIdentifier",
                      pickle.dumps(ObjectIdentifier(ObjectType.DEVICE, 1)))
        self.assertEqual(PropertyValue.date(2026, 3, 21, 6).__reduce__()[1],
                         (2026, 3, 21, 6))
        # Containers of values deep-copy and pickle too.
        state = {"points": [AI_1, PropertyValue.real(21.5),
                            BACnetTimeStamp.sequence_number(7)]}
        self.assertEqual(copy.deepcopy(state), state)
        self.assertEqual(pickle.loads(pickle.dumps(state)), state)

    def test_the_rebuild_paths_refuse_what_no_pickle_holds(self) -> None:
        from_octets: Any = BACnetTimeStamp._from_octets  # type: ignore[attr-defined]
        typed_element: Any = PropertyValue._typed_element  # type: ignore[attr-defined]
        stamp = bytes.fromhex("1900")  # sequence number 0
        self.assertEqual(from_octets(stamp), BACnetTimeStamp.sequence_number(0))
        self.assertEqual(typed_element("timestamp", stamp).value,
                         BACnetTimeStamp.sequence_number(0))
        for octets in (b"", b"\x19", stamp + stamp):
            with self.subTest(octets=octets):
                with self.assertRaises(ValueError):
                    from_octets(octets)
                with self.assertRaises(ValueError):
                    typed_element("timestamp", octets)
        with self.assertRaises(ValueError):
            typed_element("real", stamp)
        # Both take bytes, what a pickle holds, and nothing else.
        for wrong in (bytearray(stamp), list(stamp), "1900"):
            with self.subTest(wrong=type(wrong).__name__):
                with self.assertRaises(TypeError):
                    from_octets(wrong)
                with self.assertRaises(TypeError):
                    typed_element("timestamp", wrong)
        # A caller's tag shows in the error cut to 64 characters.
        with self.assertRaises(ValueError) as raised:
            typed_element("x" * 10_000, stamp)
        self.assertIn("x" * 64 + "…", str(raised.exception))
        self.assertNotIn("x" * 65, str(raised.exception))


def frame(apdu: bytes) -> bytes:
    """A B/IP original-unicast frame carrying `apdu` with no NPDU options."""
    payload = b"\x01\x00" + apdu
    return b"\x81\x0a" + (len(payload) + 4).to_bytes(2, "big") + payload


class TypedReadCopyPickleTests(unittest.IsolatedAsyncioTestCase):
    """Typed constructed reads (#1310, #1344, #1345), whole and by index,
    copy with their element production and octets."""

    async def test_typed_reads_copy_and_pickle(self) -> None:
        server = BACnetServer(9500, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1")
        server.add_analog_input(1, "AI-1")
        server.add_schedule(1, "SCHED-1")
        server.add_calendar(1, "CAL-1")
        server.add_accumulator(1, "kWh", 70, scale=2.5, prescale=(5, 100))
        server.add_access_rights(1, "Rights", accompaniment=REMOTE_USER)
        await server.start()

        async def stop() -> None:
            await server.stop()

        self.addAsyncCleanup(stop)
        application = PropertyValue.application_data
        # Christmas 2026 at priority 3, 08:00 REAL 21.0; 2026 onwards, open.
        christmas = bytes.fromhex("0e0c7e0c19050f2eb4080000004441a800002f3903")
        await server.write_property_local(
            SCHED_1, P.EXCEPTION_SCHEDULE, PropertyValue.list([application(christmas)]),
            source_object=None)
        await server.write_property_local(
            SCHED_1, P.EFFECTIVE_PERIOD, application(bytes.fromhex("a47e010104a4ffffffff")),
            source_object=None)
        await server.write_property_local(
            CAL_1, P.DATE_LIST, PropertyValue.list([application(bytes.fromhex("0c7e0c1905"))]),
            source_object=None)
        # (object, property, index, writable): a writable value's copy is
        # written back and must read back unchanged.
        reads = [
            (AI_1, P.EVENT_TIME_STAMPS, None, False),
            (AI_1, P.EVENT_TIME_STAMPS, 2, False),
            (AI_1, P.EVENT_ALGORITHM_INHIBIT_REF, None, True),
            (SCHED_1, P.WEEKLY_SCHEDULE, None, True),
            (SCHED_1, P.EXCEPTION_SCHEDULE, None, True),
            (SCHED_1, P.EXCEPTION_SCHEDULE, 1, False),
            (SCHED_1, P.EFFECTIVE_PERIOD, None, True),
            (CAL_1, P.DATE_LIST, None, True),
            (ACC_1, P.SCALE, None, False),
            (ACC_1, P.PRESCALE, None, False),
            (RIGHTS_1, P.ACCOMPANIMENT, None, False),
        ]
        tags = set()
        for oid, prop, index, writable in reads:
            with self.subTest(prop=prop, index=index):
                value = await server.read_property(oid, prop, index)
                tags.add(value.tag)
                for copied in copies(value):
                    self.assertEqual(copied, value)
                    self.assertEqual(copied.tag, value.tag)
                    self.assertEqual(copied.value, value.value)
                # The typed form, mappings of ObjectIdentifier, PropertyValue
                # and BACnetTimeStamp, copies as well.
                for copied in copies(value.value):
                    self.assertEqual(copied, value.value)
                if writable:
                    await server.write_property_local(
                        oid, prop, pickle.loads(pickle.dumps(value)), source_object=None)
                    self.assertEqual(await server.read_property(oid, prop, index), value)
        self.assertLessEqual({"list", "timestamp", "special_event", "date_range", "scale",
                              "prescale", "device_object_reference",
                              "object_property_reference"}, tags)

    async def test_a_peer_timestamp_outside_the_constructor_ranges_copies(self) -> None:
        # Month 0 and hour 99: BACnetTimeStamp.date_time refuses both, but a
        # peer can send them.
        octets = bytes.fromhex("2ea47e000000b4636363632f")
        loop = asyncio.get_running_loop()
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            address = f"127.0.0.1:{peer.getsockname()[1]}"
            async with BACnetClient(interface="127.0.0.1", port=0,
                                    apdu_timeout_ms=2000) as client:
                read = asyncio.ensure_future(
                    client.read_property(address, AI_1, P.EVENT_TIME_STAMPS, 1))
                wire, remote = await asyncio.wait_for(loop.sock_recvfrom(peer, 2048), 2)
                ack = bytes([0x30, wire[8], 12]) + wire[10:] + b"\x3e" + octets + b"\x3f"
                await loop.sock_sendto(peer, frame(ack), remote)
                value = await asyncio.wait_for(read, 2)
        self.assertEqual(value.tag, "timestamp")
        stamp = value.value
        self.assertEqual(stamp.value, ((2026, 0, 0, 0), (99, 99, 99, 99)))
        with self.assertRaises(ValueError):
            BACnetTimeStamp.date_time(*stamp.value)
        for copied in copies(value):
            self.assertEqual(copied, value)
        for copied in copies(stamp):
            self.assertEqual(copied, stamp)
            self.assertEqual(copied.value, stamp.value)


class OtherClassTests(unittest.IsolatedAsyncioTestCase):
    """The other classes hold live or receive-time state: copying or
    pickling one raises TypeError at once, on every protocol, rather than
    leaving a pickle that fails to load."""

    def assert_refused(self, value: Any) -> None:
        for protocol in range(pickle.HIGHEST_PROTOCOL + 1):
            with self.subTest(cls=type(value).__name__, protocol=protocol):
                with self.assertRaisesRegex(TypeError, "cannot pickle"):
                    pickle.dumps(value, protocol)
        for operation in (copy.copy, copy.deepcopy):
            with self.assertRaises(TypeError):
                operation(value)

    async def test_the_other_classes_refuse_copy_and_pickle(self) -> None:
        self.assert_refused(rusty_bacnet.ScHubCertificateBinding(
            uuid=bytes(range(1, 17)), allowed_vmacs=[bytes(range(1, 7))],
            leaf_sha256=[bytes(range(1, 33))]))
        server = BACnetServer(9502, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1")
        server.add_analog_input(1, "AI-1")
        self.assert_refused(server)
        await server.start()

        async def stop() -> None:
            await server.stop()

        self.addAsyncCleanup(stop)
        endpoint = rusty_bacnet.BipEndpoint(device_instance=9503, interface="127.0.0.1",
                                            broadcast_address="127.0.0.1", port=0)
        self.assert_refused(endpoint)
        await endpoint.start()

        async def close() -> None:
            await endpoint.close()

        self.addAsyncCleanup(close)
        self.assert_refused(await endpoint.client())
        self.assert_refused(await endpoint.server())
        address = await server.local_address()
        async with BACnetClient(interface="127.0.0.1", port=0,
                                apdu_timeout_ms=2000) as client:
            self.assert_refused(client)
            notifications = await client.cov_notifications()
            self.assert_refused(notifications)
            await client.subscribe_cov(address, 1, AI_1, confirmed=False, lifetime=60)
            self.assert_refused(await asyncio.wait_for(notifications.__anext__(), 3))
            await client.who_is_directed(address)
            for _ in range(100):
                devices = await client.discovered_devices()
                if devices:
                    break
                await asyncio.sleep(0.02)
            self.assertTrue(devices)
            self.assert_refused(devices[0])


if __name__ == "__main__":
    unittest.main()
