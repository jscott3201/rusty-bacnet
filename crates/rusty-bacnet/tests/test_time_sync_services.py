"""The time-synchronization bits of the Device's Protocol_Services_Supported
(#2617).

A server whose time-synchronization policy is off, or on with a source
allowlist that names nobody, lists neither TimeSynchronization nor
UTCTimeSynchronization, and one whose policy is on lists both, limited or
not, while it accepts some source (Clause 12.11.14 names what the device
executes; Clauses 16.7 and 16.8 define the services). A Python
endpoint's policy is off unless it is given. The reads are raw
ReadProperty and ReadPropertyMultiple exchanges over a B/IP socket, and the
reply octets are compared. The policy is fixed when the server starts:
nothing sets it afterwards, so a read has no change to follow.
"""
import unittest

from rusty_bacnet import BACnetServer, BipEndpoint
from test_endpoint_cov import Subscriber

INSTANCE = 2617
READ_PROPERTY, READ_PROPERTY_MULTIPLE = 12, 14
# Bits 32 (TimeSynchronization) and 36 (UTCTimeSynchronization) of the
# 49-bit string: the first and fifth bits of its fifth octet.
OCTET, LOCAL, UTC = 4, 0x80, 0x08
# ReadProperty of the Device's Protocol_Services_Supported (property 97).
READ_SERVICES = "0c 02 00 0a 39 19 61"
# ReadPropertyMultiple of the same, named twice.
READ_SERVICES_TWICE = "0c 02 00 0a 39 1e 09 61 09 61 1f"
VALUE = bytes([0x85, 0x08, 0x07])


def services_value(octets):
    """The seven data octets after the bit string's tag, length and
    unused-bits octets, in the first occurrence of `octets`."""
    start = octets.index(VALUE) + len(VALUE)
    return octets[start:start + 7]


class TimeSyncServicesTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.peer = Subscriber()
        self.addCleanup(self.peer.close)

    async def services(self, to):
        """The Protocol_Services_Supported octets by ReadProperty and by
        ReadPropertyMultiple, which must agree."""
        await self.peer.send(to, 1, READ_PROPERTY, READ_SERVICES)
        reply = await self.peer.apdu()
        self.assertEqual(reply[:3], bytes([0x30, 1, READ_PROPERTY]))
        # Object identifier, property, opening tag, the value, closing tag.
        self.assertEqual(reply[3:11], bytes.fromhex("0c 02 00 0a 39 19 61 3e".replace(" ", "")))
        self.assertEqual(reply[11:14], VALUE)
        data = reply[14:21]
        self.assertEqual(reply[21:], b"\x3f")
        await self.peer.send(to, 2, READ_PROPERTY_MULTIPLE, READ_SERVICES_TWICE)
        reply = await self.peer.apdu()
        self.assertEqual(reply[:3], bytes([0x30, 2, READ_PROPERTY_MULTIPLE]))
        self.assertEqual(reply.count(VALUE + data), 2, reply.hex())
        return data

    async def server_services(self, policy):
        keywords = {} if policy is Ellipsis else {"time_sync_policy": policy}
        server = BACnetServer(INSTANCE, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1", **keywords)
        await server.start()

        async def stop():
            await server.stop()

        self.addAsyncCleanup(stop)
        return await self.services(await server.local_address())

    async def endpoint_services(self, **keywords):
        async with BipEndpoint(INSTANCE, interface="127.0.0.1", port=0,
                               broadcast_address="127.0.0.1", **keywords) as owner:
            return await self.services(await owner.local_address())

    def assertBits(self, data, listed):
        self.assertEqual((data[OCTET] & LOCAL != 0, data[OCTET] & UTC != 0), (listed, listed),
                         data.hex())

    @staticmethod
    def without_time_sync(data):
        rest = bytearray(data)
        rest[OCTET] &= ~(LOCAL | UTC)
        return bytes(rest)

    async def test_a_servers_policy_decides_the_bits(self):
        default = await self.server_services(Ellipsis)
        self.assertBits(default, True)
        self.assertEqual(default[1] & 0x08, 0x08, "ReadProperty is listed")
        for name, policy, listed in (
            ("None", None, True),
            ("empty", {}, True),
            ("off", {"enabled": False}, False),
            # On, but with every sender refused: the device executes the
            # service for no peer.
            ("allowlist of nobody", {"source_restriction": []}, False),
            # On, with senders limited: the device executes the service for
            # the peers it accepts.
            ("allowlist of one", {"source_restriction": [(None, bytes(6))]}, True),
            ("restricted", {"max_step_ms": 0, "max_sources": 1,
                            "per_source_rate": (1.0, 1), "global_rate": (1.0, 1),
                            "coalesce_window_ms": 1000}, True),
        ):
            with self.subTest(policy=name):
                data = await self.server_services(policy)
                self.assertBits(data, listed)
                self.assertEqual(self.without_time_sync(data), self.without_time_sync(default),
                                 "no other bit moves")

    async def test_an_endpoint_lists_the_bits_only_when_given_a_policy(self):
        on = await self.endpoint_services(time_sync_policy={})
        self.assertBits(on, True)
        for name, keywords, listed in (
            ("none given", {}, False),
            ("None", {"time_sync_policy": None}, False),
            ("off", {"time_sync_policy": {"enabled": False}}, False),
            ("empty", {"time_sync_policy": {}}, True),
            ("mutation policy, no time sync", {"mutation_policy": "permissive"}, False),
            ("allowlist of nobody", {"time_sync_policy": {"source_restriction": []}}, False),
            ("allowlist of one",
             {"time_sync_policy": {"source_restriction": [(None, bytes(6))]}}, True),
        ):
            with self.subTest(endpoint=name):
                data = await self.endpoint_services(**keywords)
                self.assertBits(data, listed)
                self.assertEqual(self.without_time_sync(data), self.without_time_sync(on),
                                 "no other bit moves")


if __name__ == "__main__":
    unittest.main()
