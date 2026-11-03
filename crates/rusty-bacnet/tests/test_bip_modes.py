"""B/IP modes from Python (#938): bbmd= and foreign_device=, on loopback."""
import ast
import asyncio
import socket
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import (
    BACnetClient, BACnetServer, Bbmd, BipEndpoint, ForeignDevice, ObjectIdentifier,
    ObjectType, PropertyIdentifier as P, PropertyValue as V,
)


async def eventually(check, seconds=5.0):
    """Poll the coroutine function ``check`` until it returns a true value."""
    deadline = asyncio.get_running_loop().time() + seconds
    while True:
        result = await check()
        if result:
            return result
        if asyncio.get_running_loop().time() > deadline:
            raise AssertionError("condition not met in time")
        await asyncio.sleep(0.02)


def stub_fields(name):
    """The fields the installed stub's TypedDict ``name`` declares."""
    stub = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    tree = ast.parse(stub.read_text(encoding="utf-8"))
    typed = next(node for node in tree.body
                 if isinstance(node, ast.ClassDef) and node.name == name)
    return {node.target.id for node in typed.body if isinstance(node, ast.AnnAssign)}


def free_udp_port():
    """A UDP port on 127.0.0.1 that was free a moment ago."""
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


async def accepted_registration(client):
    """Wait for the registration observations asserted by the loopback test."""
    async def accepted():
        status = await client.foreign_registration()
        # The lease may survive while a later renewal is still pending.
        return status if (status["registered"]
                          and status["last_outcome"] == "accepted") else None
    return await eventually(accepted, seconds=20)


class RegistrationWaitTests(unittest.IsolatedAsyncioTestCase):
    async def test_waits_for_a_live_lease_and_an_accepted_attempt(self):
        accepted = {"registered": True, "last_outcome": "accepted",
                    "last_result": 0, "accepted": 2, "lease_remaining": 59.0}
        observations = iter([
            {"registered": False, "last_outcome": "pending",
             "last_result": None, "accepted": 0, "lease_remaining": None},
            # An accepted attempt alone does not establish a live lease.
            {"registered": False, "last_outcome": "accepted",
             "last_result": 0, "accepted": 1, "lease_remaining": None},
            # A renewal can be pending while an earlier lease remains live.
            {"registered": True, "last_outcome": "pending",
             "last_result": None, "accepted": 1, "lease_remaining": 49.0},
            accepted,
        ])

        class Client:
            async def foreign_registration(self):
                return next(observations)

        status = await accepted_registration(Client())
        self.assertEqual(status, accepted)


class OptionTests(unittest.TestCase):
    def test_foreign_device_keeps_its_settings(self):
        foreign = ForeignDevice("192.0.2.10", ttl=600, renewal=20)
        self.assertEqual((foreign.bbmd, foreign.ttl, foreign.renewal),
                         ("192.0.2.10:47808", 600, 20.0))
        self.assertIsNone(ForeignDevice("192.0.2.10:47809").renewal)
        self.assertEqual(ForeignDevice("192.0.2.10:47809").ttl, 300)

    def test_foreign_device_refuses_bad_settings(self):
        for kwargs in ({"bbmd": "no where"}, {"bbmd": "192.0.2.10:0"},
                       {"bbmd": "bbmd.example:0"}, {"bbmd": "-bbmd"},
                       {"bbmd": "999.1.1.1"},
                       {"bbmd": "0.0.0.0"}, {"bbmd": "255.255.255.255"},
                       {"bbmd": "224.0.0.1"}, {"bbmd": "192.0.2.10", "ttl": 0},
                       {"bbmd": "192.0.2.10", "ttl": 60, "renewal": 0},
                       {"bbmd": "192.0.2.10", "ttl": 60, "renewal": 60},
                       {"bbmd": "192.0.2.10", "ttl": 60, "renewal": float("nan")},
                       {"bbmd": "192.0.2.10", "ttl": 60, "renewal": 0.05}):
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                ForeignDevice(**kwargs)
        self.assertEqual(ForeignDevice("192.0.2.10", ttl=60, renewal=0.1).renewal, 0.1)
        for ttl in (-1, 65536):
            with self.subTest(ttl=ttl), self.assertRaises(OverflowError):
                ForeignDevice("192.0.2.10", ttl=ttl)

    def test_a_bbmd_may_be_a_host_name(self):
        # B/IP only (#2308): looked up when the owner starts.
        self.assertEqual(ForeignDevice("bbmd.example").bbmd, "bbmd.example:47808")
        named = ForeignDevice("bbmd.example:47809", ttl=60, renewal=20)
        self.assertEqual((named.bbmd, named.ttl, named.renewal),
                         ("bbmd.example:47809", 60, 20))
        self.assertEqual(repr(ForeignDevice("bbmd", ttl=60)),
                         "ForeignDevice(bbmd='bbmd:47808', ttl=60, renewal=None)")
        # An address keeps its own messages.
        with self.assertRaisesRegex(ValueError, "nonzero port"):
            ForeignDevice("192.0.2.10:0")

    def test_bbmd_keeps_its_settings(self):
        bbmd = Bbmd(["10.0.5.2", ("10.0.6.2:47809", "255.255.255.0")],
                    accept_foreign_devices=True, management_acl=["10.0.0.9"],
                    bdt_persist_path="bdt.bin")
        self.assertEqual(bbmd.bdt, [("10.0.5.2:47808", "255.255.255.255"),
                                    ("10.0.6.2:47809", "255.255.255.0")])
        self.assertTrue(bbmd.accept_foreign_devices)
        self.assertEqual(bbmd.management_acl, ["10.0.0.9"])
        self.assertEqual(str(bbmd.bdt_persist_path), "bdt.bin")
        default = Bbmd()
        self.assertEqual((default.bdt, default.accept_foreign_devices,
                          default.management_acl, default.bdt_persist_path),
                         ([], False, [], None))

    def test_bbmd_alone_on_its_subnet_is_off_by_default(self):
        # A BBMD alone behind its NAT skips the local rebroadcast (#1668).
        self.assertFalse(Bbmd().alone_on_subnet)
        self.assertTrue(Bbmd(alone_on_subnet=True).alone_on_subnet)

    def test_bbmd_behind_nat_keeps_its_global_address(self):
        # A BBMD behind a NAT router (#940).
        self.assertIsNone(Bbmd().nat_global_address)
        self.assertEqual(Bbmd().nat_private_ranges, [])
        bbmd = Bbmd(["203.0.113.1:47809", "198.51.100.2:47809"],
                    nat_global_address="203.0.113.1:47809",
                    nat_private_ranges=["100.64.9.9/10"])
        self.assertEqual(bbmd.nat_global_address, "203.0.113.1:47809")
        self.assertEqual(bbmd.nat_private_ranges, ["100.64.0.0/10"])
        self.assertEqual(Bbmd(nat_global_address="203.0.113.1")
                         .nat_global_address, "203.0.113.1:47808")
        # No peers: a BBMD that serves foreign devices only.
        self.assertEqual(Bbmd([], nat_global_address="203.0.113.1:47809").bdt, [])
        self.assertIn("nat_global_address='203.0.113.1:47809'", repr(bbmd))
        for kwargs in ({"nat_global_address": "203.0.113"},
                       {"nat_global_address": "203.0.113.1:0"},
                       {"nat_global_address": "239.1.1.1"},
                       {"nat_private_ranges": ["10.0.0.0/8"]},
                       {"nat_global_address": "203.0.113.1",
                        "nat_private_ranges": ["100.64.0.0/33"]},
                       {"nat_global_address": "203.0.113.1",
                        "nat_private_ranges": ["100.64.0.0"]},
                       {"nat_global_address": "203.0.113.1",
                        "nat_private_ranges": [f"100.{i}.0.0/16" for i in range(5)]},
                       {"nat_global_address": "127.0.0.1:47809"},
                       {"bdt": ["198.51.100.2:47809"],
                        "nat_global_address": "203.0.113.1:47809"},
                       {"bdt": ["203.0.113.1:47809",
                                ("198.51.100.0:47809", "255.255.255.0")],
                        "nat_global_address": "203.0.113.1:47809"}):
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                Bbmd(**kwargs)

    def test_bbmd_refuses_bad_settings(self):
        for kwargs in ({"bdt": ["10.0.5"]}, {"bdt": [("10.0.5.2", "mask")]},
                       {"bdt": [42]}, {"management_acl": ["10.0.0"]}):
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                Bbmd(**kwargs)

    def test_the_modes_are_mutually_exclusive_and_b_ip_only(self):
        both = {"bbmd": Bbmd(), "foreign_device": ForeignDevice("192.0.2.10")}
        with self.assertRaises(ValueError):
            BACnetClient(**both)
        with self.assertRaises(ValueError):
            BACnetServer(3100, **both)
        with self.assertRaises(ValueError):
            BipEndpoint(3100, **both)
        for mode in ({"bbmd": Bbmd()}, {"foreign_device": ForeignDevice("192.0.2.10")}):
            with self.subTest(mode=list(mode)):
                with self.assertRaises(ValueError):
                    BACnetClient(transport="ipv6", **mode)
                with self.assertRaises(ValueError):
                    BACnetServer(3100, transport="ipv6", **mode)
                # One mode alone is accepted.
                BACnetClient(**mode)
                BACnetServer(3100, **mode)
                BipEndpoint(3100, **mode)


class LoopbackTests(unittest.IsolatedAsyncioTestCase):
    async def test_a_foreign_client_discovers_a_server_through_its_bbmd(self):
        server = BACnetServer(3101, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1",
                              bbmd=Bbmd(accept_foreign_devices=True))
        await server.start()
        try:
            address = await server.local_address()
            foreign = ForeignDevice(address, ttl=60, renewal=10)
            async with BACnetClient(interface="127.0.0.1", port=0,
                                    foreign_device=foreign) as client:
                status = await accepted_registration(client)
                self.assertEqual(status["last_outcome"], "accepted")
                self.assertEqual(status["last_result"], 0)
                self.assertEqual(status["last_bbmd"], address)
                self.assertGreaterEqual(status["accepted"], 1)
                self.assertEqual((status["refused"], status["lapses"]), (0, 0))
                # A lease counts down from the send, not this observation.
                self.assertGreater(status["lease_remaining"], 0)
                self.assertLessEqual(status["lease_remaining"], 60)
                # Renewed every 10 s, not at half the 60 s TTL.
                self.assertLessEqual(status["next_attempt_in"], 10.0)

                # The Who-Is goes to the BBMD, which hands it to its own
                # device; the I-Am comes back forwarded.
                async def discovered():
                    await client.who_is(3101, 3101)
                    await asyncio.sleep(0.2)
                    return await client.get_device(3101)
                device = await eventually(discovered)
                self.assertEqual(device.object_identifier,
                                 ObjectIdentifier(ObjectType.DEVICE, 3101))
            self.assertEqual((await server.foreign_registration())["attempts"], 0)
        finally:
            await server.stop()

    async def test_a_bbmd_named_by_host_name_takes_the_registration(self):
        # localhost stands for 127.0.0.1 on every host; B/IP keeps the IPv4
        # answers (#2308).
        server = BACnetServer(3104, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1",
                              bbmd=Bbmd(accept_foreign_devices=True))
        await server.start()
        try:
            address = await server.local_address()
            port = address.rsplit(":", 1)[1]
            foreign = ForeignDevice(f"localhost:{port}", ttl=60, renewal=10)
            async with BACnetClient(interface="127.0.0.1", port=0,
                                    foreign_device=foreign) as client:
                status = await accepted_registration(client)
                self.assertEqual(status["last_bbmd"], address)
                # The name was looked up once, at start, and resolved.
                self.assertEqual(
                    (status["bbmd_lookups"], status["bbmd_lookup_failures"],
                     status["last_bbmd_lookup_failure"], status["waiting_for_bbmd"]),
                    (1, 0, None, False))
                # Every key the stub declares, and no other.
                self.assertEqual(set(status), stub_fields("ForeignRegistrationStatus"))
        finally:
            await server.stop()

    async def test_a_host_name_that_does_not_resolve_still_starts(self):
        # The .invalid domain never resolves (RFC 2606): the client starts,
        # and registration waits for an address that never comes.
        foreign = ForeignDevice("bbmd.invalid", ttl=60)
        async with BACnetClient(interface="127.0.0.1", port=0,
                                foreign_device=foreign) as client:
            status = await client.foreign_registration()
            self.assertEqual((status["attempts"], status["registered"]), (0, False))
            self.assertIsNone(status["last_outcome"])
            self.assertTrue(status["waiting_for_bbmd"])

    async def test_a_failed_lookup_is_counted_and_named(self):
        # localhost stands only for the client's own address here, which it
        # can't register with: the lookup fails at once, with no name
        # service involved, and the client keeps waiting for its BBMD.
        port = free_udp_port()
        foreign = ForeignDevice(f"localhost:{port}", ttl=60)
        async with BACnetClient(interface="127.0.0.1", port=port,
                                foreign_device=foreign) as client:
            async def failed():
                status = await client.foreign_registration()
                return status if status["bbmd_lookup_failures"] else None
            status = await eventually(failed)
            self.assertEqual(
                (status["bbmd_lookups"], status["last_bbmd_lookup_failure"],
                 status["waiting_for_bbmd"], status["attempts"]),
                (1, "unusable_address", True, 0))

    async def test_a_refused_registration_is_reported(self):
        # A BBMD that takes no foreign devices refuses the registration.
        server = BACnetServer(3102, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1", bbmd=Bbmd())
        await server.start()
        try:
            address = await server.local_address()
            endpoint = BipEndpoint(3103, interface="127.0.0.1", port=0,
                                   foreign_device=ForeignDevice(address, ttl=60))
            with self.assertRaises(RuntimeError):
                await endpoint.foreign_registration()
            async with endpoint:
                async def refused():
                    status = await endpoint.foreign_registration()
                    return status if status["refused"] else None
                status = await eventually(refused, seconds=20)
                self.assertFalse(status["registered"])
                self.assertEqual(status["last_outcome"], "refused")
                self.assertEqual(status["last_result"], 0x0030)
                self.assertEqual((status["accepted"], status["renewal_hints"]), (0, 0))
        finally:
            await server.stop()

    async def test_a_bbmd_server_reports_its_ip_mode(self):
        # A registered port needs a broadcast address other than its own.
        server = BACnetServer(3104, interface="127.0.0.1", port=0,
                              broadcast_address="127.255.255.255", registered_network_port=1,
                              bbmd=Bbmd(accept_foreign_devices=True))
        server.add_bip_network_port(1, "bbmd", ip_address="127.0.0.1", udp_port=0)
        oid = ObjectIdentifier(ObjectType.NETWORK_PORT, 1)
        await server.start()
        try:
            address = await server.local_address()
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                self.assertEqual(await client.read_property(address, oid, P.BACNET_IP_MODE),
                                 V.enumerated(2))
                self.assertEqual(
                    await client.read_property(address, oid, P.BBMD_ACCEPT_FD_REGISTRATIONS),
                    V.boolean(True))
                # Not a foreign device: nothing registers.
                status = await client.foreign_registration()
                self.assertEqual((status["attempts"], status["registered"]), (0, False))
        finally:
            await server.stop()

    async def test_status_needs_a_running_owner(self):
        client = BACnetClient(interface="127.0.0.1", port=0)
        with self.assertRaises(RuntimeError):
            await client.foreign_registration()
        server = BACnetServer(3105, interface="127.0.0.1", port=0)
        with self.assertRaises(RuntimeError):
            await server.foreign_registration()


if __name__ == "__main__":
    unittest.main()
