"""share_wildcard_port: a B/IP port is claimed unless sharing is asked for (#2366)."""
import asyncio
import socket
import sys
import unittest

from rusty_bacnet import BACnetClient, BACnetServer, BipEndpoint


def holder():
    """A plain socket on a port the OS picks, as another program would hold it."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("0.0.0.0", 0))
    return sock


def owners(port, share):
    """A client, a server and an endpoint on 127.0.0.1 at ``port``."""
    return [
        BACnetClient(interface="127.0.0.1", port=port, share_wildcard_port=share),
        BACnetServer(2366, interface="127.0.0.1", port=port, share_wildcard_port=share),
        BipEndpoint(2367, interface="127.0.0.1", port=port, share_wildcard_port=share),
    ]


class ShareWildcardPortTests(unittest.IsolatedAsyncioTestCase):
    def test_only_b_ip_takes_it(self):
        with self.assertRaises(ValueError):
            BACnetServer(2360, transport="ipv6", share_wildcard_port=True)
        with self.assertRaises(ValueError):
            BACnetClient(transport="ipv6", share_wildcard_port=True)

    async def test_a_held_port_fails_the_start_and_says_why(self):
        # A claiming owner's error names the keyword; a sharing one's says
        # the holder doesn't share. Windows lets a sharing bind in beside a
        # socket that set no options, so it checks the first only.
        cases = [(False, "share_wildcard_port")]
        if sys.platform != "win32":
            cases.append((True, "refuses sharing"))
        with holder() as held:
            port = held.getsockname()[1]
            for share, says in cases:
                for owner in owners(port, share):
                    # A client starts as its context opens.
                    start = owner.__aenter__ if isinstance(owner, BACnetClient) else owner.start
                    with self.subTest(owner=type(owner).__name__, share=share):
                        with self.assertRaises(Exception) as raised:
                            await asyncio.wait_for(start(), 10)
                        self.assertIn(says, str(raised.exception))


if __name__ == "__main__":
    unittest.main()
