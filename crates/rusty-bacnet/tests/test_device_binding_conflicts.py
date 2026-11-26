"""The server's device binding conflicts counter (#2825); loopback only."""
from __future__ import annotations

import ast
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import BACnetServer


class DeviceBindingConflictsTests(unittest.IsolatedAsyncioTestCase):
    def test_the_stub_declares_it(self):
        stub = Path(rusty_bacnet.__file__).with_suffix(".pyi")
        tree = ast.parse(stub.read_text())
        server = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == "BACnetServer")
        method = next(n for n in server.body if isinstance(n, ast.FunctionDef) and n.name == "device_binding_conflicts")
        self.assertEqual(ast.unparse(method.returns), "Awaitable[int]")

    async def test_zero_at_start_and_refused_outside_a_run(self):
        server = BACnetServer(123, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")
        with self.assertRaisesRegex(RuntimeError, "server not started"):
            await server.device_binding_conflicts()
        await server.start()
        try:
            self.assertEqual(await server.device_binding_conflicts(), 0)
        finally:
            await server.stop()
        with self.assertRaisesRegex(RuntimeError, "server not started"):
            await server.device_binding_conflicts()


if __name__ == "__main__":
    unittest.main()
