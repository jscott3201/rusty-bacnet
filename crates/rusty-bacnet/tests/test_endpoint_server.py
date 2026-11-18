"""``await endpoint.server()`` (#2617): the ``BACnetServer`` the endpoint's
session owns, on its one address.

The read-only stance is checked on the wire with a raw B/IP socket, as raw
PDUs: a peer's WriteProperty, CreateObject and DeleteObject are denied with
SERVICES / SERVICE_REQUEST_DENIED and a TimeSynchronization leaves the clock
alone, while reads and discovery are answered. The handle's lifetime is the
endpoint's: its ``start()`` and ``stop()`` are refused, the endpoint's close
stops it, and a handle from before a restart never reaches the new server.
"""
import asyncio
import datetime
import unittest

from rusty_bacnet import (
    BACnetClient, BACnetServer, BipEndpoint, ObjectIdentifier, ObjectType,
    PropertyIdentifier, PropertyValue,
)
from test_endpoint_cov import DENIED, Subscriber
from test_time_sync_policy import (
    TIME_SYNCHRONIZATION, UTC, UTC_TIME_SYNCHRONIZATION, time_sync_frame, udp_socket,
)

INSTANCE = 2617
DEVICE = ObjectIdentifier(ObjectType.DEVICE, INSTANCE)
AI1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
PV = PropertyIdentifier.PRESENT_VALUE
WRITE_PROPERTY, CREATE_OBJECT, DELETE_OBJECT, READ_PROPERTY = 15, 10, 11, 12
# AI-1's Out_Of_Service written TRUE; an Analog Value created by type; AI-1
# deleted; AI-1's Present_Value read.
REQUESTS = {
    WRITE_PROPERTY: "0c 00 00 00 01 19 51 3e 11 3f",
    CREATE_OBJECT: "0e 09 02 0f",
    DELETE_OBJECT: "c4 00 00 00 01",
}
READ_PV = "0c 00 00 00 01 19 55"


def endpoint(**keywords):
    """A B/IP endpoint on loopback with AI-1 at 3.0."""
    owner = BipEndpoint(INSTANCE, interface="127.0.0.1", port=0,
                        broadcast_address="127.0.0.1", **keywords)
    owner.add_analog_input(1, "AI 1", present_value=3.0)
    return owner


class EndpointServerWireTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.peer = Subscriber()
        self.addCleanup(self.peer.close)

    async def test_the_read_only_stance_denies_writes_creates_and_deletes(self):
        async with endpoint() as owner:
            to = await owner.local_address()
            for invoke_id, (service, body) in enumerate(REQUESTS.items(), start=1):
                with self.subTest(service=service):
                    await self.peer.send(to, invoke_id, service, body)
                    reply = await self.peer.apdu()
                    self.assertEqual(reply[:3], bytes([0x50, invoke_id, service]))
                    self.assertIn(DENIED, reply)
            # Nothing changed: AI-1 is there and in service, no AV appeared.
            await self.peer.send(to, 9, READ_PROPERTY, READ_PV)
            reply = await self.peer.apdu()
            self.assertEqual(reply[:3], bytes([0x30, 9, READ_PROPERTY]))
            server = await owner.server()
            self.assertEqual(await server.read_property(AI1, PropertyIdentifier.OUT_OF_SERVICE),
                             PropertyValue.boolean(False))
            objects = await server.read_property(DEVICE, PropertyIdentifier.OBJECT_LIST)
            self.assertNotIn(ObjectType.ANALOG_VALUE,
                             {oid.object_type for oid in objects.value})

    async def test_a_given_policy_takes_the_same_requests(self):
        async with endpoint(mutation_policy="permissive") as owner:
            to = await owner.local_address()
            await self.peer.send(to, 1, WRITE_PROPERTY, REQUESTS[WRITE_PROPERTY])
            self.assertEqual(await self.peer.apdu(), bytes([0x20, 1, WRITE_PROPERTY]))
            await self.peer.send(to, 2, CREATE_OBJECT, REQUESTS[CREATE_OBJECT])
            # A ComplexACK naming the new object.
            self.assertEqual((await self.peer.apdu())[:3], bytes([0x30, 2, CREATE_OBJECT]))

    async def settled(self, server, admitted):
        """Wait until the server has admitted at least `admitted` unconfirmed
        requests and runs none; the admitted total."""
        async with asyncio.timeout(3):
            while True:
                counters = await server.request_admission_counters()
                if (counters["unconfirmed_admitted_total"] >= admitted
                        and counters["unconfirmed_active"] == 0):
                    return counters["unconfirmed_admitted_total"]
                await asyncio.sleep(0.01)

    async def year_after_a_time_sync(self, owner, service, year):
        """Send a time sync to `year` in `service` and read the Device's year."""
        server = await owner.server()
        admitted = await self.settled(server, 0)
        sock = udp_socket()
        self.addCleanup(sock.close)
        ip, port = (await owner.local_address()).rsplit(":", 1)
        when = datetime.datetime(year, 2, 3, 10, tzinfo=UTC)
        await asyncio.get_running_loop().sock_sendto(
            sock, time_sync_frame(when, service=service), (ip, int(port)))
        await self.settled(server, admitted + 1)
        return (await server.read_property(DEVICE, PropertyIdentifier.LOCAL_DATE)).value[0]

    async def test_time_synchronization_is_off_whatever_the_mutation_policy(self):
        # Off unless asked for (maintainer decision on #2617), unlike
        # BACnetServer()'s default: under the read-only stance, a permissive
        # or deny-all policy and a callable, which is never asked about it.
        asked = []

        def callable_policy(context):
            asked.append(context.service)
            return True

        for policy in (None, "permissive", "deny_all", callable_policy):
            async with endpoint(mutation_policy=policy) as owner:
                for service, year in ((TIME_SYNCHRONIZATION, 2001),
                                      (UTC_TIME_SYNCHRONIZATION, 2002)):
                    with self.subTest(policy=policy, service=service):
                        self.assertNotEqual(
                            await self.year_after_a_time_sync(owner, service, year), year)
        self.assertEqual(asked, [])

    async def test_a_time_sync_policy_turns_it_on_in_both_services(self):
        # `{}` takes every valid request, as BACnetServer's default does; the
        # mutation policy has no say in it.
        for policy in (None, "deny_all"):
            async with endpoint(mutation_policy=policy, time_sync_policy={}) as owner:
                for service, year in ((TIME_SYNCHRONIZATION, 2001),
                                      (UTC_TIME_SYNCHRONIZATION, 2002)):
                    with self.subTest(policy=policy, service=service):
                        self.assertEqual(
                            await self.year_after_a_time_sync(owner, service, year), year)

    async def test_the_endpoint_is_discoverable_and_lists_the_servers_services(self):
        # The narrow responder answered no Who-Is and listed ReadProperty
        # alone (#2617).
        async with endpoint() as owner:
            address = await owner.local_address()
            async with BACnetClient(interface="127.0.0.1", port=0,
                                    broadcast_address="127.0.0.1",
                                    apdu_timeout_ms=2000) as client:
                found = await client.discover(scope=address, low=INSTANCE, high=INSTANCE,
                                              wait=2)
            self.assertEqual([row.instance for row in found], [INSTANCE])
            server = await owner.server()
            services = (await server.read_property(
                DEVICE, PropertyIdentifier.PROTOCOL_SERVICES_SUPPORTED)).value.names
            for service in ("READ_PROPERTY", "READ_PROPERTY_MULTIPLE", "WRITE_PROPERTY",
                            "SUBSCRIBE_COV", "WHO_IS", "I_AM"):
                self.assertIn(service, services)
            self.assertGreater((await owner.status())["server_seat_apdus"], 0)


class EndpointServerLifetimeTests(unittest.IsolatedAsyncioTestCase):
    async def test_the_handle_is_a_bacnet_server_whose_endpoint_starts_and_stops_it(self):
        owner = endpoint()
        async with owner:
            server = await owner.server()
            self.assertIsInstance(server, BACnetServer)
            self.assertEqual(await server.read_property(AI1, PV), PropertyValue.real(3.0))
            for call in (server.start, server.stop):
                with self.subTest(call=call.__name__):
                    with self.assertRaisesRegex(RuntimeError, "is the endpoint's"):
                        await call()
            with self.assertRaisesRegex(RuntimeError, "takes its objects from the endpoint"):
                server.add_analog_value(2, "Late")
            # Refused, and still serving.
            await server.write_property_local(AI1, PropertyIdentifier.OUT_OF_SERVICE,
                                              PropertyValue.boolean(True), source_object=None)
            self.assertEqual(await server.read_property(AI1, PropertyIdentifier.OUT_OF_SERVICE),
                             PropertyValue.boolean(True))
        with self.assertRaisesRegex(RuntimeError, "came from is closed"):
            await asyncio.wait_for(server.read_property(AI1, PV), 5)

    async def test_first_clients_asked_together_are_one_client(self):
        # client() shares the session lock with the server's calls (#2617),
        # and its slot's own lock keeps one client per session: callers
        # racing for the first one all get it, and none is refused.
        for _ in range(20):
            async with endpoint() as owner:
                clients = await asyncio.gather(*(owner.client() for _ in range(8)))
                self.assertTrue(all(client is clients[0] for client in clients), clients)

    async def test_a_handle_from_before_a_restart_never_reaches_the_new_server(self):
        owner = endpoint()
        async with owner:
            old = await owner.server()
            await old.set_present_value_local(AI1, PropertyValue.real(8.0))
        owner.add_analog_input(1, "AI 1", present_value=3.0)
        async with owner:
            with self.assertRaisesRegex(RuntimeError, "came from is closed"):
                await old.read_property(AI1, PV)
            new = await owner.server()
            self.assertEqual(await new.read_property(AI1, PV), PropertyValue.real(3.0))

    async def test_a_close_with_server_calls_in_flight_ends_them_all(self):
        owner = endpoint()
        await owner.start()
        server = await owner.server()

        async def read():
            try:
                return await server.read_property(AI1, PV)
            except RuntimeError as error:
                return error

        calls = [asyncio.ensure_future(read()) for _ in range(50)]
        await asyncio.sleep(0)
        results = await asyncio.wait_for(asyncio.gather(owner.close(), *calls), 10)
        for result in results[1:]:
            if isinstance(result, RuntimeError):
                self.assertIn("came from is closed", str(result))
            else:
                self.assertEqual(result, PropertyValue.real(3.0))


if __name__ == "__main__":
    unittest.main()
