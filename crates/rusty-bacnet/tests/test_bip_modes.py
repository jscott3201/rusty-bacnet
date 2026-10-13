"""B/IP modes from Python (#938): bbmd= and foreign_device=, on loopback."""
import asyncio
import unittest

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


class OptionTests(unittest.TestCase):
    def test_foreign_device_keeps_its_settings(self):
        foreign = ForeignDevice("192.0.2.10", ttl=600, renewal=20)
        self.assertEqual((foreign.bbmd, foreign.ttl, foreign.renewal),
                         ("192.0.2.10:47808", 600, 20.0))
        self.assertIsNone(ForeignDevice("192.0.2.10:47809").renewal)
        self.assertEqual(ForeignDevice("192.0.2.10:47809").ttl, 300)

    def test_foreign_device_refuses_bad_settings(self):
        for kwargs in ({"bbmd": "nowhere"}, {"bbmd": "192.0.2.10:0"},
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
                async def registered():
                    status = await client.foreign_registration()
                    return status if status["registered"] else None
                status = await eventually(registered, seconds=20)
                self.assertEqual(status["last_outcome"], "accepted")
                self.assertEqual(status["last_result"], 0)
                self.assertEqual(status["last_bbmd"], address)
                self.assertGreaterEqual(status["accepted"], 1)
                self.assertEqual((status["refused"], status["lapses"]), (0, 0))
                self.assertGreater(status["lease_remaining"], 50)
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
