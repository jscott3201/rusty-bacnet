"""Where who_is() and discover() send their Who-Is (#2560).

With no scope the Who-Is is a local broadcast, which no router passes on:
its NPDU names no destination network. scope="global" and a network number
are the explicit choices. The client is a foreign device of a raw BBMD
peer, so each broadcast reaches the peer as Distribute-Broadcast-To-Network
(Annex J.5) carrying the NPDU as sent. A client's first Who-Is goes right
after the What-Is-Network-Number it asks on that first need (#2572), a
local broadcast whatever the Who-Is's scope; later ones go alone.
"""
import ast
import asyncio
import inspect
import unittest
from pathlib import Path

import rusty_bacnet
from bvll_peer import RawPeer, bvll
from rusty_bacnet import BACnetClient, BACnetRouter, BipRouterPort, ForeignDevice
from test_bip_modes import accepted_registration

# A Who-Is with no range: Unconfirmed-Request PDU, service choice 8.
WHO_IS = bytes([0x10, 0x08])
# NPDU version 1, control 0: no destination network, no reply expected.
LOCAL = bytes([0x01, 0x00]) + WHO_IS
# Control 0x20 (destination present), DNET 65535, DLEN 0, hop count 255.
GLOBAL = bytes([0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF]) + WHO_IS
# The same with DNET 5.
NETWORK_5 = bytes([0x01, 0x20, 0x00, 0x05, 0x00, 0xFF]) + WHO_IS
# What-Is-Network-Number: a network message (control 0x80), type 0x12, with
# no destination network.
ASK = bytes([0x01, 0x80, 0x12])

REGISTER_FOREIGN_DEVICE = 0x05
DISTRIBUTE_BROADCAST = 0x09


def bbmd(data, source):
    """Accept every registration with a successful BVLC-Result."""
    if len(data) >= 2 and data[0] == 0x81 and data[1] == REGISTER_FOREIGN_DEVICE:
        return [bvll(0x00, bytes([0x00, 0x00]))]
    return None


class DiscoveryScopeTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.peer = RawPeer(bbmd)
        self.addCleanup(self.peer.close)
        self.client = BACnetClient(interface="127.0.0.1", port=0,
                                   foreign_device=ForeignDevice(self.peer.address, ttl=60))
        await self.client.__aenter__()

        async def stop():
            await asyncio.wait_for(self.client.stop(), 10)
        self.addAsyncCleanup(stop)
        await accepted_registration(self.client)

    async def broadcast_npdus(self, call, count=1):
        """The NPDUs the BBMD is handed to broadcast while ``call`` runs,
        once ``count`` have come or 3 s have passed."""
        self.peer.received.clear()
        await call()
        deadline = asyncio.get_running_loop().time() + 3
        while True:
            npdus = [data[4:] for data, _ in self.peer.received
                     if data[:2] == bytes([0x81, DISTRIBUTE_BROADCAST])]
            if len(npdus) >= count or asyncio.get_running_loop().time() > deadline:
                return npdus
            await asyncio.sleep(0.02)

    async def test_no_scope_is_a_local_broadcast(self):
        client = self.client
        for name, call, sent in (("who_is", client.who_is, [ASK, LOCAL]),
                                 ("discover", lambda: client.discover(timeout_ms=0), [LOCAL]),
                                 ("who_is local", lambda: client.who_is(scope="local"), [LOCAL])):
            with self.subTest(call=name):
                self.assertEqual(await self.broadcast_npdus(call, len(sent)), sent)

    async def test_global_and_a_network_are_explicit(self):
        client = self.client
        first = True
        for scope, npdu in (("global", GLOBAL), (5, NETWORK_5)):
            for name, call in (("who_is", lambda: client.who_is(scope=scope)),
                               ("discover", lambda: client.discover(timeout_ms=0, scope=scope))):
                # Only the client's first Who-Is is asked before.
                sent = [ASK, npdu] if first else [npdu]
                first = False
                with self.subTest(call=name, scope=scope):
                    self.assertEqual(await self.broadcast_npdus(call, len(sent)), sent)

    async def test_a_bad_scope_raises_before_anything_is_sent(self):
        client = self.client
        self.peer.received.clear()
        for scope, error in (("everywhere", ValueError), ("", ValueError), (0, ValueError),
                             (65535, ValueError), (-1, ValueError), (True, TypeError),
                             (1.5, TypeError), (b"local", TypeError)):
            for name, call in (("who_is", client.who_is), ("discover", client.discover)):
                with self.subTest(call=name, scope=scope):
                    with self.assertRaisesRegex(error, "scope is 'local', 'global' or a network"):
                        call(scope=scope)
        await asyncio.sleep(0.1)
        self.assertEqual([d for d, _ in self.peer.received if d[1] == DISTRIBUTE_BROADCAST], [])


class RouterApplicationScopeTests(unittest.TestCase):
    def test_a_local_who_is_on_a_router_application_names_the_scopes_to_use(self):
        router = BACnetRouter([BipRouterPort(1, interface="127.0.0.1", port=0)],
                              application="client")
        client = BACnetClient(transport="router", router=router)
        for call in (client.who_is, client.discover):
            for scope in ({}, {"scope": "local"}):
                with self.subTest(call=call.__name__, scope=scope):
                    with self.assertRaisesRegex(ValueError, "scope=<network number>.*scope='global'"):
                        call(**scope)


class DiscoverySignatureTests(unittest.TestCase):
    """help() and inspect show the discovery methods as the stub declares
    them, scope='local' included."""

    def stub_method(self, name):
        stub = Path(rusty_bacnet.__file__).with_suffix(".pyi")
        tree = ast.parse(stub.read_text(encoding="utf-8"))
        client = next(node for node in tree.body
                      if isinstance(node, ast.ClassDef) and node.name == "BACnetClient")
        return next(node for node in client.body
                    if isinstance(node, ast.FunctionDef) and node.name == name)

    def test_runtime_and_stub_signatures_match(self):
        for name in ("who_is", "discover", "who_is_directed"):
            with self.subTest(method=name):
                runtime = inspect.signature(getattr(BACnetClient, name))
                runtime_params = [(p.name, p.default) for p in runtime.parameters.values()
                                  if p.name != "self"]
                method = self.stub_method(name)
                args = method.args.args[1:]
                defaults = [None] * (len(args) - len(method.args.defaults)) + [
                    ast.literal_eval(d) for d in method.args.defaults]
                stub_params = [(arg.arg, inspect.Parameter.empty if i < len(args) - len(method.args.defaults)
                                else default)
                               for i, (arg, default) in enumerate(zip(args, defaults))]
                self.assertEqual(runtime_params, stub_params)
        self.assertEqual(inspect.signature(BACnetClient.who_is).parameters["scope"].default, "local")


if __name__ == "__main__":
    unittest.main()
