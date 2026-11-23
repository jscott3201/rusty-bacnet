"""Where who_is() and discover() send their Who-Is (#2560), and
who_has_by_id(), who_has_by_name() and who_has_replies() their Who-Has
(#2596).

With no scope the request is a local broadcast, which no router passes on:
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
from rusty_bacnet import (BACnetClient, BACnetRouter, BipRouterPort, ForeignDevice,
                          ObjectIdentifier, ObjectType, RoutedTarget)
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

# A Who-Has for analog-value 1, by identifier: Unconfirmed-Request PDU,
# service choice 7, then the identifier under context tag 2.
AV1 = ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)
WHO_HAS_AV1 = bytes([0x10, 0x07, 0x2C, 0x00, 0x80, 0x00, 0x01])
# The NPDU headers of a local broadcast, a global one and one to DNET 5.
HEADERS = {"local": LOCAL[:2], "global": GLOBAL[:6], 5: NETWORK_5[:6]}

REGISTER_FOREIGN_DEVICE = 0x05
DISTRIBUTE_BROADCAST = 0x09


def bbmd(data, source):
    """Accept every registration with a successful BVLC-Result."""
    if len(data) >= 2 and data[0] == 0x81 and data[1] == REGISTER_FOREIGN_DEVICE:
        return [bvll(0x00, bytes([0x00, 0x00]))]
    return None


class ForeignClient(unittest.IsolatedAsyncioTestCase):
    """A started client registered as a foreign device of a raw BBMD peer."""

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


class DiscoveryScopeTests(ForeignClient):
    async def test_remote_station_without_a_route_keeps_dadr_on_link_broadcast(self):
        target = RoutedTarget(5, b"\x07")
        destination = bytes.fromhex("01 20 00 05 01 07 ff")
        frames = await self.broadcast_npdus(lambda: self.client.who_is(scope=target), count=2)
        self.assertEqual(frames, [ASK, destination + WHO_IS])
        frames = await self.broadcast_npdus(
            lambda: self.client.discover(scope=target, low=4, high=6, wait=0))
        self.assertEqual(frames, [destination + WHO_IS + bytes.fromhex("09 04 19 06")])

    async def test_remote_station_through_a_named_router_uses_original_unicast(self):
        loop = asyncio.get_running_loop()
        received = loop.create_future()

        def observe(data, source):
            if data[:2] == bytes.fromhex("81 0a"):
                loop.call_soon_threadsafe(received.set_result, data)
            return ()

        router = RawPeer(observe)
        self.addCleanup(router.close)
        target = RoutedTarget(5, b"\x07", router=router.address)
        frames = await self.broadcast_npdus(lambda: self.client.who_is(scope=target))
        self.assertEqual(frames, [ASK])
        data = await asyncio.wait_for(received, 3)
        self.assertEqual(data[4:], bytes.fromhex("01 20 00 05 01 07 ff") + WHO_IS)

    async def test_no_scope_is_a_local_broadcast(self):
        client = self.client
        for name, call, sent in (("who_is", client.who_is, [ASK, LOCAL]),
                                 ("discover", lambda: client.discover(wait=0), [LOCAL]),
                                 ("who_is local", lambda: client.who_is(scope="local"), [LOCAL])):
            with self.subTest(call=name):
                self.assertEqual(await self.broadcast_npdus(call, len(sent)), sent)

    async def test_global_and_a_network_are_explicit(self):
        client = self.client
        first = True
        for scope, npdu in (("global", GLOBAL), (5, NETWORK_5)):
            for name, call in (("who_is", lambda: client.who_is(scope=scope)),
                               ("discover", lambda: client.discover(wait=0, scope=scope))):
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
                    with self.assertRaisesRegex(error, "scope is 'local', 'global', a network"):
                        call(scope=scope)
        await asyncio.sleep(0.1)
        self.assertEqual([d for d, _ in self.peer.received if d[1] == DISTRIBUTE_BROADCAST], [])


class WhoHasScopeTests(ForeignClient):
    """The Who-Has calls take the scope the Who-Is calls take."""

    def who_has_calls(self, **scope):
        client = self.client
        return (("who_has_by_id", lambda: client.who_has_by_id(AV1, **scope)),
                ("who_has_replies", lambda: client.who_has_replies(AV1, wait=0, **scope)))

    async def test_no_scope_is_a_local_broadcast(self):
        for name, call in self.who_has_calls() + self.who_has_calls(scope="local"):
            with self.subTest(call=name):
                self.assertEqual(await self.broadcast_npdus(call), [HEADERS["local"] + WHO_HAS_AV1])
        # By name, the name follows under context tag 3 (UTF-8, its length).
        [npdu] = await self.broadcast_npdus(lambda: self.client.who_has_by_name("AV-1"))
        self.assertEqual(npdu, HEADERS["local"] + bytes([0x10, 0x07, 0x3D, 0x05, 0x00]) + b"AV-1")

    async def test_global_and_a_network_are_explicit(self):
        for scope in ("global", 5):
            calls = self.who_has_calls(scope=scope) + (
                ("who_has_by_name", lambda: self.client.who_has_by_name("AV-1", scope=scope)),)
            for name, call in calls:
                with self.subTest(call=name, scope=scope):
                    [npdu] = await self.broadcast_npdus(call)
                    self.assertEqual(npdu[:len(HEADERS[scope]) + 2], HEADERS[scope] + bytes([0x10, 0x07]))

    async def test_a_bad_scope_raises_before_anything_is_sent(self):
        client = self.client
        self.peer.received.clear()
        calls = (("who_has_by_id", lambda scope: client.who_has_by_id(AV1, scope=scope)),
                 ("who_has_by_name", lambda scope: client.who_has_by_name("AV-1", scope=scope)),
                 ("who_has_replies", lambda scope: client.who_has_replies("AV-1", scope=scope)))
        for scope, error in ((RoutedTarget(5, b"\x07"), TypeError), ("everywhere", ValueError), (0, ValueError), (65535, ValueError),
                             (True, TypeError), (1.5, TypeError)):
            for name, call in calls:
                with self.subTest(call=name, scope=scope):
                    with self.assertRaisesRegex(error, "scope is 'local', 'global', a network"):
                        call(scope)
        await asyncio.sleep(0.1)
        self.assertEqual([d for d, _ in self.peer.received if d[1] == DISTRIBUTE_BROADCAST], [])


class RouterApplicationScopeTests(unittest.TestCase):
    def test_a_station_address_on_a_router_application_names_what_to_pass(self):
        # The application is on each of the router's networks, so a local
        # scope asks them all (#2601), but a station's address names no one
        # of them: refused before anything is sent.
        router = BACnetRouter([BipRouterPort(1, interface="127.0.0.1", port=0)],
                              application="client")
        client = BACnetClient(transport="router", router=router)
        station = {"scope": "192.168.1.10"}
        for call in (client.who_is, client.discover):
            with self.subTest(call=call.__name__):
                with self.assertRaisesRegex(
                        ValueError, "names no one station for a directed Who-Is: pass a RoutedTarget"):
                    call(**station)
        oid = ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)
        for name, call in (("who_has_by_id", lambda **s: client.who_has_by_id(oid, **s)),
                           ("who_has_by_name", lambda **s: client.who_has_by_name("AV-1", **s)),
                           ("who_has_replies", lambda **s: client.who_has_replies(oid, **s))):
            with self.subTest(call=name):
                with self.assertRaisesRegex(
                        ValueError, "names no one station for a directed Who-Has: pass scope=<network number>"):
                    call(**station)
        # The local scope is the default there too.
        self.assertEqual(client.resolve_scope, "local")


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
        for name in ("who_is", "discover", "who_has_by_id", "who_has_by_name",
                     "who_has_replies"):
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
                stub_params += [(arg.arg, ast.literal_eval(default)) for arg, default
                                in zip(method.args.kwonlyargs, method.args.kw_defaults)]
                self.assertEqual(runtime_params, stub_params)
                # The range, the wait and the scope go by keyword only: the
                # calls would otherwise take them in differing positions.
                keyword_only = {p.name for p in runtime.parameters.values()
                                if p.kind is inspect.Parameter.KEYWORD_ONLY}
                self.assertEqual(keyword_only, {"low", "high", "scope", "wait"}
                                 & set(runtime.parameters))
                self.assertEqual(keyword_only, {arg.arg for arg in method.args.kwonlyargs})
        routers = inspect.signature(BACnetClient.discover_routers).parameters
        self.assertIsNone(routers["router"].default)
        self.assertIs(routers["router"].kind, inspect.Parameter.KEYWORD_ONLY)
        self.assertIs(routers["wait"].kind, inspect.Parameter.KEYWORD_ONLY)
        self.assertEqual([arg.arg for arg in self.stub_method("discover_routers").args.kwonlyargs],
                         ["router", "wait"])
        for name in ("who_is", "discover", "who_has_by_id", "who_has_by_name", "who_has_replies"):
            with self.subTest(method=name):
                scope = inspect.signature(getattr(BACnetClient, name)).parameters["scope"]
                self.assertEqual(scope.default, "local")


if __name__ == "__main__":
    unittest.main()
