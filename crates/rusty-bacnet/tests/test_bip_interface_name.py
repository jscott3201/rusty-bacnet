"""Named B/IP interfaces use the same address and port ownership as literals."""

import functools
import socket
import subprocess
import sys
import unittest

import rusty_bacnet as rb
from port_retry import lost_to_another_socket, rerun_on_lost_port
from test_share_port_by_address import free_port


@functools.cache
def loopback_name():
    if sys.platform == "win32":
        # socket.if_nameindex() on Windows exposes the index name. B/IP uses
        # the adapter's friendly name, also reported as the IP interface alias.
        command = ("[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); "
                   "(Get-NetIPAddress -AddressFamily IPv4 -IPAddress '127.0.0.1' "
                   "-ErrorAction Stop).InterfaceAlias")
        return subprocess.check_output(["powershell.exe", "-NoProfile", "-NonInteractive",
                                        "-Command", command], encoding="utf-8", timeout=30).strip()
    names = {name for _, name in socket.if_nameindex()}
    return next(name for name in ("lo0", "lo") if name in names)


class NamedBipInterfaceTests(unittest.IsolatedAsyncioTestCase):
    async def test_named_and_literal_endpoints_claim_the_same_socket(self):
        await rerun_on_lost_port(self.claim_and_rebind)

    async def claim_and_rebind(self):
        def endpoint(instance, interface, port):
            return rb.BipEndpoint(instance, interface=interface, port=port,
                                  broadcast_address="127.255.255.255", share_port_by_address=True)

        named = endpoint(2614, loopback_name(), free_port())
        async with named:
            address = await named.local_address()
            ip, port = address.split(":")
            self.assertEqual(ip, "127.0.0.1")
            literal = endpoint(2615, ip, int(port))
            with self.assertRaises(OSError) as conflict:
                await literal.start()
            self.assertTrue(lost_to_another_socket(conflict.exception), conflict.exception)
        async with literal:
            self.assertEqual(await literal.local_address(), address)

    async def test_client_server_router_and_multicast_validation(self):
        name = loopback_name()
        options = dict(interface=name, port=0, broadcast_address="127.255.255.255")
        server = rb.BACnetServer(2614, **options)
        server.add_analog_input(1, "Named interface", present_value=9.0)
        await server.start()
        try:
            address = await server.local_address()
            self.assertTrue(address.startswith("127.0.0.1:"))
            async with rb.BACnetClient(**options) as client:
                value = await client.read_property(address, rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 1),
                                                   rb.PropertyIdentifier.PRESENT_VALUE)
                self.assertEqual(value.value, 9.0)
        finally:
            await server.stop()
        self.assertEqual(rb.BipRouterPort(1, **options).interface, "127.0.0.1")
        multicast = rb.Multicast("239.192.1.14", port=47809)
        for owner, args in ((rb.BACnetClient, ()), (rb.BACnetServer, (2614,)), (rb.BipEndpoint, (2614,))):
            with self.subTest(owner=owner):
                owner(*args, **options, multicast=multicast)
                with self.assertRaisesRegex(ValueError, "unknown network interface"):
                    owner(*args, interface="__absent_nic__")
        with self.assertRaisesRegex(ValueError, "broadcast"):
            rb.BipEndpoint(2614, interface=name, broadcast_address=name)
        with self.assertRaisesRegex(ValueError, "interface"):
            rb.BipEndpoint(2614, interface="127.0.0.1/8")
