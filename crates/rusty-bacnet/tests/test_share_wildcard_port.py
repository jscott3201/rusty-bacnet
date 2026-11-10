"""share_wildcard_port: a B/IP or B/IPv6 port is claimed unless sharing is
asked for (#2366, #2385)."""
import asyncio
import socket
import sys
import unittest

from port_retry import rerun_on_lost_port
from rusty_bacnet import BACnetClient, BACnetServer, BipEndpoint


def holder(family=socket.AF_INET):
    """A plain socket on a port the OS picks, as another program would hold it."""
    sock = socket.socket(family, socket.SOCK_DGRAM)
    sock.bind(("::" if family == socket.AF_INET6 else "0.0.0.0", 0))
    return sock


def owners(port, share):
    """A client, a server and an endpoint on 127.0.0.1 at ``port``."""
    return [
        BACnetClient(interface="127.0.0.1", port=port, share_wildcard_port=share),
        BACnetServer(2366, interface="127.0.0.1", port=port, share_wildcard_port=share),
        BipEndpoint(2367, interface="127.0.0.1", port=port, share_wildcard_port=share),
    ]


def ipv6_owners(port, share):
    """A B/IPv6 client and server on ::1 at ``port``."""
    return [
        BACnetClient(transport="ipv6", ipv6_interface="::1", port=port,
                     ipv6_device_instance=2385, share_wildcard_port=share),
        BACnetServer(2386, transport="ipv6", ipv6_interface="::1", port=port,
                     share_wildcard_port=share),
    ]


def start_of(owner):
    """A client starts as its context opens."""
    return owner.__aenter__ if isinstance(owner, BACnetClient) else owner.start


class ShareWildcardPortTests(unittest.IsolatedAsyncioTestCase):
    def test_only_b_ip_and_b_ipv6_take_it(self):
        # Accepted, and nothing binds before a start.
        BACnetServer(2360, transport="ipv6", share_wildcard_port=True)
        BACnetClient(transport="ipv6", share_wildcard_port=True)
        for transport in ("sc", "mstp"):
            with self.subTest(transport=transport):
                with self.assertRaisesRegex(ValueError, "share_wildcard_port"):
                    BACnetServer(2360, transport=transport, share_wildcard_port=True)
                with self.assertRaisesRegex(ValueError, "share_wildcard_port"):
                    BACnetClient(transport=transport, share_wildcard_port=True)

    async def test_a_held_port_fails_the_start_and_says_why(self):
        # A claiming owner's error names the keyword; a sharing one's says
        # the port can't be shared. Windows decides a sharing bind beside a
        # socket that set no options by the two sockets' accounts (Microsoft's
        # SO_REUSEADDR table), so only the claiming case runs there.
        cases = [(False, "share_wildcard_port")]
        if sys.platform != "win32":
            cases.append((True, "can't be shared"))
        for family, make in [(socket.AF_INET, owners), (socket.AF_INET6, ipv6_owners)]:
            with holder(family) as held:
                port = held.getsockname()[1]
                for share, says in cases:
                    for owner in make(port, share):
                        name = type(owner).__name__
                        with self.subTest(owner=name, family=family.name, share=share):
                            with self.assertRaises(Exception) as raised:
                                await asyncio.wait_for(start_of(owner)(), 10)
                            self.assertIn(says, str(raised.exception))

    async def test_two_b_ipv6_owners_on_one_port_start_only_when_both_share(self):
        # The second owner is refused beside a claiming first, sharing or
        # not, and starts beside a sharing first where the OS lets two
        # SO_REUSEADDR sockets share a wildcard port: Linux and Windows, not
        # macOS.
        async def run():
            with holder(socket.AF_INET6) as probe:
                port = probe.getsockname()[1]
            for first_shares in (False, True):
                first = ipv6_owners(port, first_shares)[1]
                second = ipv6_owners(port, True)[0]
                # Another process can take the probed port: rerun then.
                await asyncio.wait_for(first.start(), 10)
                started = False
                try:
                    if first_shares and sys.platform != "darwin":
                        await asyncio.wait_for(second.__aenter__(), 10)
                        started = True
                    else:
                        # On macOS the first may have asked to share too:
                        # the message blames no one.
                        with self.assertRaises(Exception) as raised:
                            await asyncio.wait_for(second.__aenter__(), 10)
                            started = True
                        self.assertIn("can't be shared", str(raised.exception))
                finally:
                    if started:
                        await second.__aexit__(None, None, None)
                    await first.stop()

        await rerun_on_lost_port(run)


if __name__ == "__main__":
    unittest.main()
