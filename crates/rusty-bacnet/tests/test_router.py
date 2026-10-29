"""Real owned B/IP forwarding, cleanup barriers, and installed API parity."""
import ast
import asyncio
import gc
import inspect
import socket
import subprocess
import sys
import unittest

import rusty_bacnet as rb
from test_bip_modes import accepted_registration
from test_transport_errno import assert_addr_in_use
from test_endpoint_stub_parity import installed_stub, runtime_arg_kinds, stub_arg_kinds, stub_classes, stub_method


def port(network, **kw):
    return rb.BipRouterPort(network, interface="127.0.0.1", port=kw.pop("port", 0),
                            broadcast_address="127.0.0.1", **kw)


def exclusive(address):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    if hasattr(socket, "SO_EXCLUSIVEADDRUSE"):
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
    try:
        sock.bind(("0.0.0.0", int(address.rsplit(":", 1)[1])))
    except BaseException:
        sock.close()
        raise
    return sock


class RouterSurfaceTests(unittest.TestCase):
    def test_constructor_validation_and_immutable_modes(self):
        default = port(1)
        self.assertEqual((default.network_number, default.interface, default.port,
                          default.broadcast_address, default.mode),
                         (1, "127.0.0.1", 0, "127.0.0.1", "normal"))
        self.assertEqual(port(5, bbmd=rb.Bbmd()).mode, "bbmd")
        self.assertEqual(port(7, foreign_device=rb.ForeignDevice("127.0.0.1")).mode,
                         "foreign_device")
        for name in ("network_number", "interface", "port", "broadcast_address", "mode"):
            with self.assertRaises(AttributeError):
                setattr(default, name, None)
        for n in (0, 65535):
            with self.assertRaises(ValueError): port(n)
        for n in (-1, 65536):
            with self.assertRaises(OverflowError): port(n)
        for kwargs in ({"interface": "::1"}, {"broadcast_address": "wrong"}):
            with self.assertRaises(ValueError): rb.BipRouterPort(1, **kwargs)
        with self.assertRaises(ValueError):
            port(1, bbmd=rb.Bbmd(), foreign_device=rb.ForeignDevice("127.0.0.1"))
        for ports in ([], [port(1), port(1)], [port(n) for n in range(1,257)]):
            with self.assertRaises(ValueError): rb.BACnetRouter(ports)
        with self.assertRaises(TypeError): rb.BACnetRouter([{}])
        with self.assertRaises(TypeError): port(1, bbmd={})
        rb.BACnetRouter([port(65534)])

    def test_installed_signatures_and_snapshot_types(self):
        classes = stub_classes(installed_stub())
        for cls, methods in ((rb.BipRouterPort, ["__init__"]),
                             (rb.BACnetRouter, ["__init__", "start", "close", "ports", "routes", "__aenter__", "__aexit__"])):
            for method in methods:
                runtime = cls if method == "__init__" else getattr(cls, method)
                node = stub_method(classes[cls.__name__], method)
                self.assertEqual(runtime_arg_kinds(runtime), stub_arg_kinds(node))
                positional = node.args.posonlyargs + node.args.args
                defaults = dict(zip([arg.arg for arg in positional[-len(node.args.defaults):]],
                                    [ast.literal_eval(value) for value in node.args.defaults])) if node.args.defaults else {}
                defaults.update({arg.arg: ast.literal_eval(value)
                                 for arg, value in zip(node.args.kwonlyargs, node.args.kw_defaults)
                                 if value is not None})
                runtime_defaults = {name: value.default for name, value in inspect.signature(runtime).parameters.items()
                                    if name != "self" and value.default is not inspect.Parameter.empty}
                self.assertEqual(runtime_defaults, defaults)
                if method != "__init__":
                    self.assertIsInstance(node, ast.FunctionDef)
                    self.assertTrue(ast.unparse(node.returns).startswith("Awaitable["))
        for cls in (rb.RouterPortInfo, rb.RouterRoute, rb.RouterCleanupFailure, rb.BacnetRouterCloseError):
            self.assertIn(cls.__name__, classes)
        self.assertTrue(issubclass(rb.BacnetRouterCloseError, rb.BacnetError))

    def test_event_loop_affinity(self):
        router = rb.BACnetRouter([port(1)])
        async def first(): await router.close()
        asyncio.run(first())
        async def second():
            for method in (router.start, router.close, router.ports, router.routes, router.__aenter__):
                with self.assertRaisesRegex(RuntimeError, "different asyncio"):
                    method()
        asyncio.run(second())

    def test_interpreter_exit_with_live_router(self):
        script = '''
import asyncio
from rusty_bacnet import BACnetRouter, BipRouterPort
router = BACnetRouter([BipRouterPort(1, interface="127.0.0.1", port=0)])
async def main():
    await router.start()
    assert (await router.ports())[0].address.startswith("127.0.0.1:")
asyncio.run(main())
'''
        result = subprocess.run([sys.executable, "-W", "error", "-c", script],
                                capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")


class RouterTests(unittest.IsolatedAsyncioTestCase):
    @staticmethod
    async def cleanup(method):
        await asyncio.wait_for(method(), 10)

    async def test_close_restart_context_and_owned_snapshots(self):
        router = rb.BACnetRouter([port(1), port(5), port(7)])
        await router.close()
        for method in (router.ports, router.routes):
            with self.assertRaises(RuntimeError): await method()
        future = router.start()
        self.assertTrue(inspect.isawaitable(future))
        await asyncio.wait_for(future, 10)
        self.addAsyncCleanup(self.cleanup, router.close)
        with self.assertRaisesRegex(RuntimeError, "already started"): await router.start()
        self.assertIs(await router.__aenter__(), router)
        ports = await router.ports()
        routes = await router.routes()
        self.assertEqual([(p.index,p.network_number) for p in ports], [(0,1),(1,5),(2,7)])
        self.assertEqual([(r.network_number,r.port_index,r.is_direct,r.next_hop,r.reachability) for r in routes],
                         [(1,0,True,None,"reachable"),(5,1,True,None,"reachable"),(7,2,True,None,"reachable")])
        for p in ports:
            self.assertEqual(p.mac[:4], bytes([127,0,0,1]))
            self.assertEqual(int.from_bytes(p.mac[4:],"big"),int(p.address.rsplit(":",1)[1]))
            self.assertNotEqual(int.from_bytes(p.mac[4:],"big"),0)
            with self.assertRaises(OSError): exclusive(p.address)
            with self.assertRaises(AttributeError): p.index=9
        with self.assertRaises(AttributeError): routes[0].reachability="busy"
        await asyncio.wait_for(router.close(),10)
        await router.close()
        for p in ports:
            with exclusive(p.address): pass  # exactly one bind; no retry or delay
        self.assertEqual(routes[0].network_number,1)
        with self.assertRaises(RuntimeError): await router.ports()
        sentinel=RuntimeError("context body sentinel")
        with self.assertRaises(RuntimeError) as caught:
            async with router as same:
                self.assertIs(same,router)
                current=await router.ports()
                raise sentinel
        self.assertIs(caught.exception,sentinel)
        for p in current:
            with exclusive(p.address): pass

    async def test_second_bind_failure_rolls_back_first_and_retry_works(self):
        # The first configured socket number is held until immediately before
        # startup; any unrelated bind race remains a failure, never a retry.
        first=exclusive("127.0.0.1:0"); first_port=first.getsockname()[1]
        blocked=exclusive("127.0.0.1:0"); blocked_port=blocked.getsockname()[1]
        self.addCleanup(blocked.close)
        router=rb.BACnetRouter([port(1,port=first_port),port(5,port=blocked_port)])
        first.close()
        with self.assertRaises(rb.BacnetTransportError) as caught:
            await asyncio.wait_for(router.start(),10)
        assert_addr_in_use(self, caught.exception)
        with exclusive(f"127.0.0.1:{first_port}"): pass
        blocked.close()
        await router.start()
        self.addAsyncCleanup(self.cleanup, router.close)
        actual=await router.ports()
        self.assertEqual([int(p.address.rsplit(":",1)[1]) for p in actual],[first_port,blocked_port])
        await router.close()
        for p in actual:
            with exclusive(p.address): pass

    async def test_three_network_discovery_and_bidirectional_real_property_reads(self):
        router=rb.BACnetRouter([port(n,bbmd=rb.Bbmd(accept_foreign_devices=True,alone_on_subnet=True)) for n in (1,5,7)])
        await router.start();self.addAsyncCleanup(self.cleanup, router.close)
        ports=await router.ports()
        clients=[]; servers=[]
        oid=rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT,0)
        for n,p in zip((1,5,7),ports):
            server=rb.BACnetServer(185800+n,interface="127.0.0.1",port=0,
                                  foreign_device=rb.ForeignDevice(p.address,ttl=60))
            server.add_analog_input(0,f"Network {n} temperature",62,10.25+n)
            await server.start();self.addAsyncCleanup(self.cleanup, server.stop)
            await accepted_registration(server);servers.append(server)
            client=rb.BACnetClient(interface="127.0.0.1",port=0,
                                  foreign_device=rb.ForeignDevice(p.address,ttl=60))
            await client.__aenter__();self.addAsyncCleanup(self.cleanup, client.stop)
            await accepted_registration(client);clients.append(client)
        [all_routes]=await clients[0].discover_routers(timeout_ms=1500)
        self.assertEqual((all_routes.mac,all_routes.networks),(ports[0].mac,[5,7]))
        [specific]=await clients[0].discover_routers(7,timeout_ms=1500)
        self.assertIn(7,specific.networks)
        self.assertEqual(await clients[0].discover_routers(9,timeout_ms=500),[])
        # Each target has a real server with an independently configured value.
        # Device source_network proves the cache does not collapse into a direct
        # loopback read; requests and responses must traverse the router.
        for ingress,target_index in ((0,1),(0,2),(1,0),(2,1)):
            target=(1,5,7)[target_index];client=clients[ingress]
            discovered=await client.discover(timeout_ms=1500,low_limit=185800+target,high_limit=185800+target)
            found=[d for d in discovered if d.object_identifier.instance==185800+target]
            self.assertEqual(len(found),1)
            self.assertEqual(found[0].source_network,target)
            value=await asyncio.wait_for(client.read_property_from_device(185800+target,oid,rb.PropertyIdentifier.PRESENT_VALUE),10)
            self.assertAlmostEqual(value.value,10.25+target)
        await router.close()
        for p in ports:
            with exclusive(p.address):pass

    async def test_destructor_requests_cleanup(self):
        # A native lifecycle probe establishes disposal's causal ownership.
        # Here only exercise the Python destructor while the shared runtime is
        # alive; explicit await is the documented joined barrier elsewhere.
        router=rb.BACnetRouter([port(1)])
        await router.start()
        del router
        gc.collect()
