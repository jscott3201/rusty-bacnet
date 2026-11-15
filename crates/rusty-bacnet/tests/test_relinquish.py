"""Relinquish and the priority array from Python (#2561).

``Device.relinquish`` writes NULL at one slot of an object's
Priority_Array and reads the slots back; ``priority_array`` reads them. Both
go through the high-level client, so the client's ``write_policy`` refuses
priorities 1, 2, 5 and 6 unless it allows them, while ``write_property``
stays unguarded. The tests run a server in this process and reach it
through a UDP relay that records each confirmed request it passes (its
service and its service data) and can go silent part-way.
"""

from __future__ import annotations

import ast
import asyncio
import copy
import inspect
import pickle
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import (
    AbortReason,
    BACnetClient,
    BACnetRouter,
    BACnetServer,
    BacnetAbortError,
    BacnetError,
    BacnetNoPriorityArrayError,
    BacnetProtocolError,
    BacnetReadBackError,
    BacnetTimeoutError,
    BacnetWriteDeniedError,
    Device,
    ErrorClass,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PriorityView,
    PropertyIdentifier,
    PropertyValue,
    Value,
    WritePolicy,
    Written,
)

import test_sc_hub_mtls as mtls
from test_router_application import bbmd_port, cleanup

DEVICE = 2561
AO = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)
BO = ObjectIdentifier(ObjectType.BINARY_OUTPUT, 5)
# A value object without a Priority_Array, and an input, whose type has none.
PLAIN = ObjectIdentifier(ObjectType.ANALOG_VALUE, 3)
AI = ObjectIdentifier(ObjectType.ANALOG_INPUT, 4)
PV = PropertyIdentifier.PRESENT_VALUE

READ_PROPERTY_MULTIPLE = 14
WRITE_PROPERTY = 15
PRESENT_VALUE = 85
GUARDED = (1, 2, 5, 6)


class Relay(asyncio.DatagramProtocol):
    """Passes datagrams between one client and the server, recording each
    confirmed request the client sends, by invoke ID, and dropping those
    after ``passing``. A retransmission of a request already seen is passed
    or dropped as the first copy was, and not recorded again."""

    def __init__(self, server: tuple[str, int], passing: int) -> None:
        self.server = server
        self.passing = passing
        self.seen: dict[int, bool] = {}
        self.requests: list[tuple[int, bytes]] = []
        self.client: tuple[str, int] | None = None
        self.front: asyncio.DatagramTransport | None = None
        self.back: asyncio.DatagramTransport | None = None

    def datagram_received(self, data: bytes, addr: tuple[str, int]) -> None:
        self.client = addr
        # BVLL (4 octets), the NPDU's version and control, then the APDU: a
        # local confirmed request has no NPDU addresses; its invoke ID is
        # the APDU's third octet and its service the fourth.
        if len(data) > 9 and data[5] & 0x28 == 0 and data[6] >> 4 == 0:
            invoke_id = data[8]
            if invoke_id not in self.seen:
                passes = len(self.requests) < self.passing
                self.seen[invoke_id] = passes
                self.requests.append((data[9], bytes(data[10:])))
            if not self.seen[invoke_id]:
                return
        assert self.back is not None
        self.back.sendto(data, self.server)

    @property
    def services(self) -> list[int]:
        return [service for service, _ in self.requests]

    def clear(self) -> None:
        self.requests.clear()


class Back(asyncio.DatagramProtocol):
    def __init__(self, relay: Relay) -> None:
        self.relay = relay

    def datagram_received(self, data: bytes, addr: tuple[str, int]) -> None:
        if self.relay.client is not None and self.relay.front is not None:
            self.relay.front.sendto(data, self.relay.client)


async def relay_to(address: str, passing: int = 1_000_000) -> Relay:
    host, port = address.rsplit(":", 1)
    loop = asyncio.get_running_loop()
    relay = Relay((host, int(port)), passing)
    relay.front, _ = await loop.create_datagram_endpoint(
        lambda: relay, local_addr=("127.0.0.1", 0)
    )
    relay.back, _ = await loop.create_datagram_endpoint(
        lambda: Back(relay), local_addr=("127.0.0.1", 0)
    )
    return relay


def relay_address(relay: Relay) -> str:
    assert relay.front is not None
    host, port = relay.front.get_extra_info("sockname")[:2]
    return f"{host}:{port}"


def close(relay: Relay) -> None:
    for transport in (relay.front, relay.back):
        if transport is not None:
            transport.close()


def write_request(data: bytes) -> tuple[int, bytes, int | None]:
    """A WriteProperty request's property, value octets and priority. The
    tests write whole properties with a one-octet property identifier."""
    assert data[0] == 0x0C, data.hex()  # object identifier, context 0
    assert data[5] == 0x19, data.hex()  # property identifier, context 1
    prop = data[6]
    assert data[7] == 0x3E, data.hex()  # opening tag 3
    end = data.rindex(0x3F)
    value, tail = data[8:end], data[end + 1 :]
    if not tail:
        return prop, value, None
    assert tail[0] == 0x49 and len(tail) == 2, data.hex()  # context 4, one octet
    return prop, value, tail[1]


def lab() -> BACnetServer:
    server = BACnetServer(
        DEVICE,
        "Plant Controller",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )
    server.add_analog_output(1, "Damper")
    server.add_binary_output(5, "Pump Start")
    server.add_analog_value(3, "Setpoint", present_value_access="writable")
    server.add_analog_input(4, "Zone Temp")
    return server


def client(**keywords) -> BACnetClient:
    """A client on loopback whose local broadcasts stay on loopback, as the
    suite's other loopback clients' do: the default broadcast address,
    255.255.255.255, has no route from 127.0.0.1 on some hosts (CI's macOS
    runner), where a local Who-Is then fails with EHOSTUNREACH."""
    defaults = {"interface": "127.0.0.1", "port": 0, "broadcast_address": "127.0.0.1",
                "apdu_timeout_ms": 2000}
    return BACnetClient(**{**defaults, **keywords})


class Lab(unittest.IsolatedAsyncioTestCase):
    passing = 1_000_000

    async def asyncSetUp(self) -> None:
        self.server = lab()
        await self.server.start()
        self.direct = await self.server.local_address()
        self.relay = await relay_to(self.direct, self.passing)
        self.address = relay_address(self.relay)

    async def asyncTearDown(self) -> None:
        close(self.relay)
        await self.server.stop()

    async def command(self, client: BACnetClient, priority: int, value: float) -> None:
        """Put `value` in the analog output's slot at `priority`, straight
        to the server, past the relay."""
        await client.write_property(
            self.direct, AO, PV, PropertyValue.real(value), priority=priority
        )


class PriorityArrayTests(Lab):
    async def test_the_slots_in_one_request(self) -> None:
        async with client() as c:
            await self.command(c, 8, 72.5)
            await self.command(c, 12, 50.0)
            view = await (await c.device(self.address)).priority_array(AO)
        self.assertEqual(self.relay.services, [READ_PROPERTY_MULTIPLE])
        self.assertIsInstance(view, PriorityView)
        self.assertEqual(view.object_id, AO)
        self.assertEqual(len(view.slots), 16)
        held = {i + 1: slot.text for i, slot in enumerate(view.slots) if slot is not None}
        self.assertEqual(held, {8: "72.5", 12: "50"})
        self.assertIs(view.slot(8), view.slots[7])
        self.assertIsNone(view.slot(9))
        self.assertEqual(view.in_control, 8)
        self.assertIsInstance(view.present_value, Value)
        self.assertEqual(view.present_value.text, "72.5")
        self.assertEqual(view.present_value.value.value, 72.5)
        self.assertEqual(view.present_value.data, 72.5)
        self.assertEqual(view.relinquish_default.text, "0")
        self.assertEqual(
            repr(view),
            "PriorityView(analog-output 1, in_control=8, present_value='72.5', "
            "relinquish_default='0', slots={8: '72.5', 12: '50'})",
        )
        self.assertEqual(repr(view.present_value), "Value('72.5')")
        self.assertEqual(str(view.present_value), "72.5")

    async def test_every_slot_empty(self) -> None:
        async with client() as c:
            view = await (await c.device(self.address)).priority_array(AO)
        self.assertEqual(view.slots, (None,) * 16)
        self.assertIsNone(view.in_control)
        self.assertEqual(view.present_value.text, view.relinquish_default.text)

    async def test_a_binary_slot_names_its_state(self) -> None:
        async with client() as c:
            await c.write_property(
                self.direct, BO, PV, PropertyValue.enumerated(1), priority=10
            )
            view = await (await c.device(self.address)).priority_array(BO)
        slot = view.slot(10)
        self.assertEqual(slot.text, "active")
        self.assertEqual(slot.data, {"value": 1, "name": "active"})
        # `.value` is what read_property of the slot gives.
        async with client() as c:
            read = await c.read_property(
                self.direct, BO, PropertyIdentifier.PRIORITY_ARRAY, 10
            )
        self.assertEqual(slot.value, read)
        self.assertEqual(view.present_value.text, "active")
        self.assertEqual(view.relinquish_default.text, "inactive")


class RelinquishTests(Lab):
    async def test_a_null_at_that_slot_then_one_read(self) -> None:
        async with client() as c:
            await self.command(c, 8, 72.5)
            await self.command(c, 12, 50.0)
            after = await (await c.device(self.address)).relinquish(AO, 8)
            self.assertEqual(self.relay.services, [WRITE_PROPERTY, READ_PROPERTY_MULTIPLE])
            prop, value, priority = write_request(self.relay.requests[0][1])
            self.assertEqual(self.relay.requests[0][1][1:5], bytes([0x00, 0x40, 0x00, 0x01]))
            self.assertEqual((prop, value, priority), (PRESENT_VALUE, b"\x00", 8))
            # The next slot down takes over.
            self.assertIsNone(after.slot(8))
            self.assertEqual(after.in_control, 12)
            self.assertEqual(after.present_value.text, "50")
            # Releasing the last command leaves Relinquish_Default.
            self.relay.clear()
            last = await (await c.device(self.address)).relinquish(AO, priority=12)
            self.assertEqual(self.relay.services, [WRITE_PROPERTY, READ_PROPERTY_MULTIPLE])
            self.assertEqual(write_request(self.relay.requests[0][1])[2], 12)
        self.assertEqual(last.slots, (None,) * 16)
        self.assertIsNone(last.in_control)
        self.assertEqual(last.present_value.text, last.relinquish_default.text)

    async def test_the_priority_is_required(self) -> None:
        async with client() as c:
            with self.assertRaises(TypeError):
                (await c.device(self.address)).relinquish(AO)  # type: ignore[call-arg]
        self.assertEqual(self.relay.requests, [])

    async def test_bad_priorities_raise_before_anything_is_sent(self) -> None:
        async with client() as c:
            for bad in (0, 17, -1, 2**70):
                with self.subTest(priority=bad), self.assertRaisesRegex(ValueError, "1 to 16"):
                    (await c.device(self.address)).relinquish(AO, bad)
            for bad in (True, False, 8.0, "8", None):
                with self.subTest(priority=bad), self.assertRaisesRegex(TypeError, "int"):
                    (await c.device(self.address)).relinquish(AO, bad)
            with self.assertRaises(ValueError):
                c.device("not an address")
            # The object is an ObjectIdentifier or its text, nothing else.
            with self.assertRaises(TypeError):
                (await c.device(self.address)).relinquish(5, 8)
            with self.assertRaises(ValueError):
                (await c.device(self.address)).relinquish("not an object", 8)
        self.assertEqual(self.relay.requests, [])

    async def test_an_address_the_link_cannot_carry_is_refused(self) -> None:
        async with client() as c:
            # Colon-separated hex of another length than six octets is no
            # address to the grammar: three parts are refused when the call
            # is made. (Two decimal parts, "10:20", are instance 20 on
            # network 10, not a two-octet MAC.)
            for target in ("00:04:d2", "0a:0b", "01:02:03:04"):
                with self.subTest(target=target), self.assertRaises(ValueError):
                    c.device(target)
            # Octets the grammar can't judge, a B/IPv6 VMAC's length and an
            # MS/TP station's, are refused for this link on await.
            for mac in (b"\x00\x04\xd2", b"\x05"):
                with self.subTest(mac=mac), self.assertRaises(ValueError):
                    await c.device((0, mac))
            # An MS/TP station is spelled "mstp:5" (a bare "5" is device
            # instance 5); this B/IP link refuses it, naming the keyword.
            with self.assertRaisesRegex(ValueError, r"MS/TP.*transport='mstp'"):
                await c.device("mstp:5")
        self.assertEqual(self.relay.requests, [])

    async def test_an_object_type_without_priorities_sends_nothing(self) -> None:
        async with client() as c:
            with self.assertRaisesRegex(ValueError, "Priority_Array"):
                await (await c.device(self.address)).relinquish(AI, 8)
            with self.assertRaisesRegex(ValueError, "Priority_Array"):
                await (await c.device(self.address)).priority_array(AI)
        self.assertEqual(self.relay.requests, [])

    async def test_an_object_without_a_priority_array(self) -> None:
        async with client() as c:
            with self.assertRaises(BacnetNoPriorityArrayError) as read:
                await (await c.device(self.address)).priority_array(PLAIN)
            self.assertEqual(self.relay.services, [READ_PROPERTY_MULTIPLE])
            self.relay.clear()
            with self.assertRaises(BacnetNoPriorityArrayError) as released:
                await (await c.device(self.address)).relinquish(PLAIN, 8)
            self.assertEqual(self.relay.services, [WRITE_PROPERTY, READ_PROPERTY_MULTIPLE])
        for caught in (read.exception, released.exception):
            self.assertIsInstance(caught, BacnetProtocolError)
            self.assertEqual(caught.error_class, ErrorClass.PROPERTY)
            self.assertEqual(caught.error_code, ErrorCode.UNKNOWN_PROPERTY)
            self.assertEqual(caught.object_id, PLAIN)
            self.assertIn("analog-value 3", str(caught))
        self.assertIsNone(read.exception.written)
        written = released.exception.written
        self.assertIsInstance(written, Written)
        self.assertEqual(
            (written.object_id, written.property_id, written.array_index, written.priority),
            (PLAIN, PV, None, 8),
        )
        self.assertEqual(written.address, self.address)
        self.assertEqual(written.value.text, "null")
        self.assertEqual(written.value.value, PropertyValue.null())
        self.assertIn("acknowledged and changed nothing", str(released.exception))

    async def test_the_device_refusing_the_write(self) -> None:
        missing = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 99)
        async with client() as c:
            with self.assertRaises(BacnetProtocolError) as caught:
                await (await c.device(self.address)).relinquish(missing, 8)
        self.assertNotIsInstance(caught.exception, BacnetNoPriorityArrayError)
        self.assertEqual(caught.exception.error_code, ErrorCode.UNKNOWN_OBJECT)
        self.assertEqual(self.relay.services, [WRITE_PROPERTY])


class PolicyTests(Lab):
    async def test_guarded_priorities_are_refused_with_nothing_sent(self) -> None:
        async with client() as c:
            self.assertEqual(c.write_policy, WritePolicy())
            for priority in GUARDED:
                with self.subTest(priority=priority):
                    with self.assertRaises(BacnetWriteDeniedError) as caught:
                        await (await c.device(self.address)).relinquish(AO, priority)
                    denied = caught.exception
                    self.assertIsInstance(denied, BacnetError)
                    self.assertNotIsInstance(denied, BacnetProtocolError)
                    self.assertEqual(denied.priority, priority)
                    self.assertIs(denied.relinquish, True)
                    self.assertEqual((denied.object_id, denied.property_id), (AO, PV))
                    self.assertIsNone(denied.array_index)
                    self.assertIn(
                        f"write_policy=WritePolicy().allow_priorities([{priority}])",
                        str(denied),
                    )
            # The other twelve go.
            await (await c.device(self.address)).relinquish(AO, 7)
        self.assertEqual(self.relay.services, [WRITE_PROPERTY, READ_PROPERTY_MULTIPLE])

    async def test_allowed_after_opting_in(self) -> None:
        policy = WritePolicy().allow_priorities(GUARDED)
        async with client(write_policy=policy) as c:
            self.assertEqual(c.write_policy, policy)
            for priority in GUARDED:
                with self.subTest(priority=priority):
                    await self.command(c, priority, 10.0 + priority)
                    self.relay.clear()
                    after = await (await c.device(self.address)).relinquish(AO, priority)
                    self.assertEqual(
                        self.relay.services, [WRITE_PROPERTY, READ_PROPERTY_MULTIPLE]
                    )
                    self.assertEqual(write_request(self.relay.requests[0][1])[2], priority)
                    self.assertIsNone(after.slot(priority))
        # One priority allowed leaves the other three refused.
        async with client(write_policy=WritePolicy().allow_priorities([5])) as c:
            await (await c.device(self.address)).relinquish(AO, 5)
            with self.assertRaises(BacnetWriteDeniedError):
                await (await c.device(self.address)).relinquish(AO, 6)

    async def test_write_property_stays_unguarded(self) -> None:
        async with client() as c:
            await c.write_property(self.address, AO, PV, PropertyValue.real(9.0), priority=1)
            self.assertEqual(self.relay.services, [WRITE_PROPERTY])
            self.assertEqual(write_request(self.relay.requests[0][1])[2], 1)
            view = await (await c.device(self.address)).priority_array(AO)
            self.assertEqual(view.in_control, 1)
            with self.assertRaises(BacnetWriteDeniedError):
                await (await c.device(self.address)).relinquish(AO, 1)
            # The low-level write releases it as before.
            await c.write_property(self.address, AO, PV, PropertyValue.null(), priority=1)
            self.assertIsNone((await (await c.device(self.address)).priority_array(AO)).in_control)

    def test_the_policy_object(self) -> None:
        policy = WritePolicy()
        self.assertEqual(WritePolicy.GUARDED, GUARDED)
        self.assertEqual(policy.refused, GUARDED)
        self.assertEqual(repr(policy), "WritePolicy(refused=(1, 2, 5, 6))")
        opened = policy.allow_priorities([1, 2])
        self.assertEqual(opened.refused, (5, 6))
        self.assertEqual(policy.refused, GUARDED, "allow_priorities gives a new policy")
        self.assertTrue(opened.allows(1) and not opened.allows(5) and policy.allows(16))
        self.assertEqual(opened, WritePolicy().allow_priorities((2, 1)))
        self.assertEqual(hash(opened), hash(WritePolicy().allow_priorities([1, 2])))
        self.assertNotEqual(opened, policy)
        self.assertNotEqual(policy, GUARDED)
        with self.assertRaises(AttributeError):
            policy.refused = ()  # type: ignore[misc]
        # The class is immutable too: its GUARDED can't be swapped out.
        with self.assertRaises(TypeError):
            WritePolicy.GUARDED = ()  # type: ignore[misc]
        self.assertEqual(WritePolicy.GUARDED, GUARDED)
        # A copy is the same policy, never the default (test_value_copy_pickle
        # covers every protocol).
        for copied in (copy.copy(opened), copy.deepcopy(opened), pickle.loads(pickle.dumps(opened))):
            self.assertEqual(copied.refused, (5, 6))
        with self.assertRaisesRegex(ValueError, "1 to 16"):
            policy.allow_priorities([1, 0])
        with self.assertRaisesRegex(TypeError, "int"):
            policy.allow_priorities([True])
        with self.assertRaises(TypeError):
            policy.allow_priorities(1)
        with self.assertRaisesRegex(ValueError, "1 to 16"):
            policy.allows(17)
        with self.assertRaises(TypeError):
            BACnetClient(write_policy=[1, 2])  # type: ignore[arg-type]
        self.assertEqual(BACnetClient().write_policy, WritePolicy())
        self.assertEqual(BACnetClient(write_policy=opened).write_policy, opened)


class ReadBackTests(Lab):
    # The WriteProperty passes; the read after it goes unanswered.
    passing = 1

    async def test_written_but_not_read_back(self) -> None:
        async with client(apdu_timeout_ms=300) as c:
            await self.command(c, 8, 72.5)
            with self.assertRaises(BacnetReadBackError) as caught:
                await (await c.device(self.address)).relinquish(AO, 8)
            # It took effect: the slot is empty now.
            view = await (await c.device(self.direct)).priority_array(AO)
        error = caught.exception
        self.assertNotIsInstance(error, BacnetTimeoutError)
        self.assertIsInstance(error.__cause__, BacnetTimeoutError)
        self.assertEqual((error.written.object_id, error.written.priority), (AO, 8))
        self.assertIn("acknowledged the relinquish of analog-output 1", str(error))
        self.assertEqual(self.relay.services, [WRITE_PROPERTY, READ_PROPERTY_MULTIPLE])
        self.assertIsNone(view.slot(8))


class NoAnswerTests(Lab):
    passing = 0

    async def test_no_answer_to_the_write(self) -> None:
        async with client(apdu_timeout_ms=300) as c:
            with self.assertRaises(BacnetTimeoutError) as caught:
                await (await c.device(self.address)).relinquish(AO, 8)
            self.assertNotIsInstance(caught.exception, BacnetReadBackError)
            self.assertEqual(self.relay.services, [WRITE_PROPERTY])
            with self.assertRaises(BacnetTimeoutError):
                await (await c.device(self.address)).priority_array(AO)
            # The typed calls report the same silence as the transaction
            # state machine's abort, as the docs say.
            with self.assertRaises(BacnetAbortError) as written:
                await c.write_property(
                    self.address, AO, PV, PropertyValue.null(), priority=8
                )
            with self.assertRaises(BacnetAbortError) as read:
                await c.read_property(self.address, AO, PV)
        for typed in (written.exception, read.exception):
            self.assertEqual(typed.reason, AbortReason.TSM_TIMEOUT)
            self.assertNotIsInstance(typed, BacnetTimeoutError)


class CancelledTests(Lab):
    # The WriteProperty passes; the read after it goes unanswered, for long.
    passing = 1

    async def test_stop_after_a_relinquish_cancelled_mid_call(self) -> None:
        c = client(apdu_timeout_ms=10_000)
        await c.__aenter__()
        try:
            pending = asyncio.ensure_future((await c.device(self.address)).relinquish(AO, 8))
            async with asyncio.timeout(10):
                while len(self.relay.requests) < 2:
                    await asyncio.sleep(0.01)
            self.assertEqual(self.relay.services, [WRITE_PROPERTY, READ_PROPERTY_MULTIPLE])
            pending.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await pending
        finally:
            # The cancelled call let go of its handle, so the stop owns the
            # client and stops it, long before the read would time out.
            await asyncio.wait_for(c.stop(), 5)
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await (await c.device(self.address)).priority_array(AO)
        # The same client starts again, and its priority calls work.
        self.relay.passing = 1_000_000
        async with c:
            view = await (await c.device(self.address)).priority_array(AO)
        self.assertIsNone(view.slot(8))


class ReceivedValueTests(Lab):
    async def test_a_view_refuses_copy_and_pickle_and_its_values_copy(self) -> None:
        async with client() as c:
            view = await (await c.device(self.address)).priority_array(AO)
        with self.assertRaises(TypeError):
            pickle.dumps(view)
        with self.assertRaises(TypeError):
            copy.copy(view)
        # A Value is a value class (part 2): it copies and pickles.
        value = view.present_value
        self.assertEqual(copy.copy(value), value)
        self.assertEqual(pickle.loads(pickle.dumps(value)), value)
        with self.assertRaises(AttributeError):
            view.in_control = 3  # type: ignore[misc]


class RouterTransportTests(unittest.IsolatedAsyncioTestCase):
    async def test_a_local_address_on_a_router_client(self) -> None:
        # The router's application has no network of its own, so a local
        # address names no station it can reach: the priority calls refuse
        # it on await with nothing sent, where read_property sends and the
        # router refuses it.
        router = BACnetRouter([bbmd_port(1)], application="client")
        await router.start()
        self.addAsyncCleanup(cleanup, router.close)
        address = (await router.ports())[0].address
        async with BACnetClient(transport="router", router=router, apdu_timeout_ms=2000) as c:
            with self.assertRaisesRegex(ValueError, "transport='router' has no network"):
                await asyncio.wait_for(c.device(address), 10)
            with self.assertRaisesRegex(BacnetError, "no network of its own"):
                await asyncio.wait_for(c.read_property(address, AO, PV), 10)


class ScAddressTests(mtls.MtlsFixture):
    """On BACnet/SC a written device's address is its VMAC in hex, as the
    binding writes SC VMACs elsewhere, not six octets read as ip:port."""

    async def test_an_sc_written_names_the_vmac_in_hex(self) -> None:
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 3)
        url = await hub.url()
        sc = dict(transport="sc", sc_hub=url, sc_ca_cert=self.path("site.pem"))
        server = BACnetServer(
            3000, sc_vmac=b"\x02\0\0\0\0\x04", sc_device_uuid=mtls.SERVER_UUID,
            sc_client_cert=self.path("server.pem"), sc_client_key=self.path("server.key"),
            **sc,
        )
        server.add_analog_value(3, "Setpoint", present_value_access="writable")
        await asyncio.wait_for(server.start(), 5)
        self.addAsyncCleanup(self.stop_server, server)
        c = BACnetClient(
            sc_vmac=b"\x02\0\0\0\0\x02", sc_device_uuid=mtls.CLIENT_UUID,
            sc_client_cert=self.path("client.pem"), sc_client_key=self.path("client.key"),
            **sc,
        )
        await asyncio.wait_for(c.__aenter__(), 5)
        self.addAsyncCleanup(self.stop_server, c)
        with self.assertRaises(BacnetNoPriorityArrayError) as caught:
            await asyncio.wait_for((await c.device("02:00:00:00:00:04")).relinquish(PLAIN, 8), 5)
        self.assertEqual(caught.exception.written.address, "02:00:00:00:00:04")


def stub_class(name: str) -> ast.ClassDef:
    path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    return next(
        node for node in tree.body if isinstance(node, ast.ClassDef) and node.name == name
    )


def stub_method(class_name: str, name: str) -> ast.FunctionDef:
    return next(
        node
        for node in stub_class(class_name).body
        if isinstance(node, ast.FunctionDef) and node.name == name
    )


def stub_parameters(method: ast.FunctionDef) -> list[tuple[str, inspect._ParameterKind]]:
    kinds = [
        *[(arg.arg, inspect.Parameter.POSITIONAL_OR_KEYWORD) for arg in method.args.args],
        *[(arg.arg, inspect.Parameter.KEYWORD_ONLY) for arg in method.args.kwonlyargs],
    ]
    return [(name, kind) for name, kind in kinds if name != "self"]


def runtime_parameters(callable_: object) -> list[tuple[str, inspect._ParameterKind]]:
    return [
        (name, parameter.kind)
        for name, parameter in inspect.signature(callable_).parameters.items()
        if name != "self"
    ]


class StubParityTests(unittest.TestCase):
    def test_the_calls_match_the_stub(self) -> None:
        for name, expected in (
            ("relinquish", ["object", "priority"]),
            ("priority_array", ["object"]),
        ):
            with self.subTest(method=name):
                # On the handle only: the client has no second spelling.
                self.assertFalse(hasattr(BACnetClient, name))
                runtime = runtime_parameters(getattr(Device, name))
                self.assertEqual([n for n, _ in runtime], expected)
                self.assertEqual(runtime, stub_parameters(stub_method("Device", name)))
                # No default: a relinquish always names its slot.
                for parameter in inspect.signature(getattr(Device, name)).parameters.values():
                    self.assertIs(parameter.default, inspect.Parameter.empty)

    def test_the_write_policy_keyword_matches_the_stub(self) -> None:
        runtime = runtime_parameters(BACnetClient)
        self.assertEqual(runtime[-2], ("write_policy", inspect.Parameter.KEYWORD_ONLY))
        self.assertIsNone(inspect.signature(BACnetClient).parameters["write_policy"].default)
        self.assertEqual(runtime, stub_parameters(stub_method("BACnetClient", "__init__")))
        self.assertEqual(runtime_parameters(WritePolicy), [])
        self.assertEqual(stub_parameters(stub_method("WritePolicy", "__init__")), [])
        self.assertEqual(
            runtime_parameters(WritePolicy.allow_priorities),
            stub_parameters(stub_method("WritePolicy", "allow_priorities")),
        )

    def test_the_new_names_are_in_the_stub(self) -> None:
        for name, members in (
            ("Value", {"value", "text", "data", "raw"}),
            ("PriorityView", {"object_id", "slots", "in_control", "relinquish_default",
                              "present_value", "slot"}),
            ("Written", {"address", "object_id", "property_id", "array_index", "value",
                         "priority", "commandable"}),
            ("WritePolicy", {"allow_priorities", "allows", "refused"}),
        ):
            with self.subTest(cls=name):
                stubbed = {
                    node.name
                    for node in stub_class(name).body
                    if isinstance(node, ast.FunctionDef)
                }
                self.assertLessEqual(members, stubbed)
                runtime = getattr(rusty_bacnet, name)
                for member in members:
                    self.assertTrue(hasattr(runtime, member), f"{name}.{member}")
        for name, base in (
            ("BacnetWriteDeniedError", "BacnetError"),
            ("BacnetReadBackError", "BacnetError"),
            ("BacnetNoPriorityArrayError", "BacnetProtocolError"),
        ):
            with self.subTest(exception=name):
                node = stub_class(name)
                self.assertEqual([b.id for b in node.bases], [base])  # type: ignore[attr-defined]
                self.assertTrue(
                    issubclass(getattr(rusty_bacnet, name), getattr(rusty_bacnet, base))
                )


if __name__ == "__main__":
    unittest.main()
