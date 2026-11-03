"""Native Futures, semantic installed signatures, and protocol boundaries."""
import asyncio
import ast
import inspect
from pathlib import Path
import unittest

import rusty_bacnet as rb


def installed_classes():
    tree = ast.parse(Path(rb.__file__).with_suffix(".pyi").read_text(), feature_version=(3, 11))
    return {node.name: node for node in tree.body if isinstance(node, ast.ClassDef)}


def native_future_exports(sources):
    """Bounded rustfmt inventory: direct bridge calls or direct self forwarding.

    This is not a Rust parser. A forwarding target must independently resolve
    to a known bridge method; unrelated calls, extra work and cycles add nothing.
    """
    import re

    methods = {}
    for source in sources:
        # Any indent, so the methods a macro generates (endpoint/builders.rs)
        # count; the body ends at the closing brace on the fn's own indent.
        for method in re.finditer(r"^( +)fn (\w+).*?^\1}", source, re.M | re.S):
            methods[method.group(2)] = method.group()
    native = {name for name, body in methods.items()
              if "crate::py_async::future_into_py" in body}
    forwards = {}
    for name, body in methods.items():
        target = re.search(r"\n +self\.(\w+)\(py\)\n +}$", body)
        if target:
            forwards[name] = target.group(1)
    while resolved := {name for name, target in forwards.items() if target in native} - native:
        native.update(resolved)
    return native


class NativeFutureTests(unittest.IsolatedAsyncioTestCase):
    async def future_result(self, owner, name, future):
        try:
            self.assertTrue(asyncio.isfuture(future))
            self.assertFalse(inspect.iscoroutine(future))
            self.assertTrue(inspect.isawaitable(future))
            with self.assertRaises(TypeError):
                asyncio.create_task(future)
            self.assertIs(asyncio.ensure_future(future), future)
            result = await asyncio.wait_for(future, 5)
            method = next(n for n in installed_classes()[owner].body
                          if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)) and n.name == name)
            self.assertIsInstance(method, ast.FunctionDef, "native Future must not advertise a coroutine function")
            self.assertIsInstance(method.returns, ast.Subscript)
            self.assertEqual(ast.unparse(method.returns.value), "Awaitable")
            return result
        finally:
            if not future.done():
                future.cancel()
            await asyncio.gather(future, return_exceptions=True)

    async def test_unit_only_stop_results_are_none_without_starting(self):
        owners = (
            ("BACnetClient", rb.BACnetClient(interface="127.0.0.1", port=0)),
            ("BACnetServer", rb.BACnetServer(865, interface="127.0.0.1", port=0)),
            ("ScHub", rb.ScHub("127.0.0.1:0", "unused-cert", "unused-key",
                               b"\x02\x00\x00\x00\x00\x01", ca_cert="unused-ca",
                               device_uuid=b"\x01" * 16)),
        )
        for name, owner in owners:
            with self.subTest(owner=name):
                self.assertIsNone(await self.future_result(name, "stop", owner.stop()))

    async def test_client_native_future_and_installed_signature(self):
        async with rb.BACnetClient(interface="127.0.0.1", port=0) as client:
            result = await self.future_result("BACnetClient", "discovered_devices", client.discovered_devices())
            self.assertEqual(result, [])

    async def test_server_hub_endpoint_role_and_context_futures(self):
        server = rb.BACnetServer(858, interface="127.0.0.1", port=0)
        endpoint = rb.BipEndpoint(device_instance=859, interface="127.0.0.1", port=0)
        hub = rb.ScHub("127.0.0.1:0", "unused-cert", "unused-key", b"\x02\x00\x00\x00\x00\x01",
                       ca_cert="unused-ca", device_uuid=b"\x01" * 16)
        try:
            self.assertIsNone(await self.future_result("ScHub", "address", hub.address()))
            await self.future_result("ScHub", "stop", hub.stop())
            self.assertIsNone(await self.future_result("BACnetServer", "start", server.start()))
            address = await self.future_result("BACnetServer", "local_address", server.local_address())
            entered = await self.future_result("BipEndpoint", "__aenter__", endpoint.__aenter__())
            self.assertIs(entered, endpoint)
            role = await self.future_result("BipEndpoint", "client", endpoint.client())
            self.assertIsInstance(role, rb.EndpointClient)
            owner_role = await self.future_result("BipEndpoint", "server", endpoint.server())
            self.assertIsInstance(owner_role, rb.EndpointServer)
            oid = rb.ObjectIdentifier(rb.ObjectType.DEVICE, 858)
            value = await self.future_result("EndpointClient", "read_property",
                                            role.read_property(address, oid, rb.PropertyIdentifier.OBJECT_NAME))
            self.assertIsInstance(value, rb.PropertyValue)
            with self.assertRaises(ValueError):
                role.read_property_multiple("invalid-address", [])
            await self.future_result("BipEndpoint", "__aexit__", endpoint.__aexit__(None, None, None))
        finally:
            await endpoint.close()
            await server.stop()
            await hub.stop()

    async def test_context_and_future_resolving_to_async_iterator(self):
        server = rb.BACnetServer(860, interface="127.0.0.1", port=0)
        server.add_analog_input(1, "future-sensor", present_value=1.0)
        client = rb.BACnetClient(interface="127.0.0.1", port=0)
        try:
            await server.start()
            entered = await self.future_result("BACnetClient", "__aenter__", client.__aenter__())
            self.assertIs(entered, client)
            iterator = await self.future_result("BACnetClient", "cov_notifications", client.cov_notifications())
            self.assertIsInstance(iterator, rb.CovNotificationIterator)
            self.assertIs(iterator.__aiter__(), iterator)
            oid = rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 1)
            address = await server.local_address()
            # Subscribe produces an initial notification without changing object state.
            await client.subscribe_cov(address, 1, oid, confirmed=False, lifetime=30)
            notification = await self.future_result("CovNotificationIterator", "__anext__", iterator.__anext__())
            self.assertIsInstance(notification, rb.CovNotification)
            await client.subscribe_cov(address, 1, oid, confirmed=False, lifetime=30)

            async def read_one():
                async for item in iterator:
                    return item
                self.fail("iterator closed before the renewal notification")

            self.assertIsInstance(await asyncio.wait_for(read_one(), 5), rb.CovNotification)
            with self.assertRaises(ValueError):
                client.write_property("invalid-address", oid, rb.PropertyIdentifier.PRESENT_VALUE,
                                      rb.PropertyValue.real(2.0), priority=0)
            self.assertIsNone(await self.future_result("BACnetClient", "__aexit__", client.__aexit__(None, None, None)))
            with self.assertRaises(StopAsyncIteration):
                await asyncio.wait_for(iterator.__anext__(), 5)
        finally:
            await client.stop()
            await server.stop()


class NativeFutureInventoryTests(unittest.TestCase):
    def test_installed_awaitables_match_native_bridge_exports(self):
        """Cross-check the native exporting method set, including special protocols.

        Scope is the eight native classes using the existing Tokio Future bridge;
        this is not a Rust parser or a claim that every method ran dynamically.
        """
        source = Path(__file__).resolve().parents[1] / "src"
        groups = {
            "BACnetClient": sorted((source / "client/client_methods").glob("*.rs")),
            "BACnetServer": sorted((source / "server/server_methods").glob("*.rs")),
            "ScHub": [source / "hub.rs"],
            "EndpointClient": [source / "endpoint/roles.rs"],
            "CovNotificationIterator": [source / "types/cov.rs"],
            # endpoint/builders.rs holds the methods the three owners share.
            **{name: [source / f"endpoint/{stem}.rs", source / "endpoint/builders.rs"]
               for name, stem in (
                ("BipEndpoint", "bip"), ("ScEndpoint", "sc"), ("MstpEndpoint", "mstp"))},
        }
        classes = installed_classes()
        for name, paths in groups.items():
            with self.subTest(class_name=name):
                self.assertTrue(paths)
                native = native_future_exports(path.read_text() for path in paths)
                self.assertTrue(native)
                cls = classes[name]
                self.assertFalse(any(isinstance(n, ast.AsyncFunctionDef) for n in cls.body))
                declared = {n.name for n in cls.body if isinstance(n, ast.FunctionDef)
                            and isinstance(n.returns, ast.Subscript)
                            and ast.unparse(n.returns.value) == "Awaitable"}
                self.assertEqual(declared, native)


class NativeFutureForwardingInventoryTests(unittest.TestCase):
    def test_only_forwarding_to_a_proven_bridge_counts(self):
        source = """
    fn close(&self, py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
        crate::py_async::future_into_py(py, async { Ok(()) })
    }
    fn exit(&self, py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
        self.close(py)
    }
    fn forwarded_exit(&self, py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
        self.exit(py)
    }
    fn ordinary(&self, py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
        Ok(py.None())
    }
    fn ordinary_forward(&self, py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
        self.ordinary(py)
    }
    fn discarded_future(&self, py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
        self.close(py);
        Ok(py.None())
    }
    fn another_owner(&self, py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
        other.close(py)
    }
    fn missing_target(&self, py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
        self.missing(py)
    }
    fn cycle_a(&self, py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
        self.cycle_b(py)
    }
    fn cycle_b(&self, py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
        self.cycle_a(py)
    }
"""
        self.assertEqual(native_future_exports([source]), {"close", "exit", "forwarded_exit"})
