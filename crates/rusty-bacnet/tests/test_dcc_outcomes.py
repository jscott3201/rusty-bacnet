"""Installed-wheel acceptance of local DCC telemetry, not an audit service."""
import ast
import asyncio
import importlib.util
from pathlib import Path
import socket
import unittest

from rusty_bacnet import BACnetServer

FIELDS = {f"{name}_total" for name in (
    "accepted", "policy_denied", "password_failure", "deprecated_denied", "malformed"
)}


class DccOutcomeTests(unittest.IsolatedAsyncioTestCase):
    async def test_counter_lifetime_shape_and_independence(self):
        server = BACnetServer(123, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1", dcc_password="sentinel")
        other = BACnetServer(124, interface="127.0.0.1", port=0,
                             broadcast_address="127.0.0.1", dcc_policy="legacy_permissive")
        with self.assertRaisesRegex(RuntimeError, "^server not started$"):
            await server.dcc_outcome_counters()
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        sock.setblocking(False)
        try:
            await server.start()
            await other.start()
            self.assertEqual(await server.dcc_outcome_counters(), dict.fromkeys(FIELDS, 0))
            self.assertEqual(await other.dcc_outcome_counters(), dict.fromkeys(FIELDS, 0))
            correct = b"\x2d\x09\x00sentinel"
            cases = [(server, b"\x19\x00" + correct, "policy_denied_total"),
                     (server, b"\x19\x01" + correct, "deprecated_denied_total"),
                     (server, b"\x19\x01", "password_failure_total"),
                     (server, b"\x19\x63" + correct, "malformed_total"),
                     (server, b"\x19\x00\x2d\x02\x00\xff", "malformed_total"),
                     (other, b"\x19\x02", "accepted_total")]
            for invoke, (target, payload, outcome) in enumerate(cases, 1):
                before = await target.dcc_outcome_counters()
                ip, port = (await target.local_address()).rsplit(":", 1)
                npdu = b"\x01\x00" + bytes([0, 5, invoke, 17]) + payload
                wire = b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu
                loop = asyncio.get_running_loop()
                await loop.sock_sendto(sock, wire, (ip, int(port)))
                await asyncio.wait_for(loop.sock_recvfrom(sock, 2048), 2)
                expected = before.copy()
                expected[outcome] += 1
                self.assertEqual(await target.dcc_outcome_counters(), expected)
            self.assertEqual((await server.dcc_outcome_counters())["accepted_total"], 0)
            self.assertEqual(await other.dcc_outcome_counters(), {
                **dict.fromkeys(FIELDS, 0), "accepted_total": 1})
        finally:
            await server.stop()
            await other.stop()
            sock.close()
        with self.assertRaisesRegex(RuntimeError, "^server not started$"):
            await server.dcc_outcome_counters()

    def test_installed_stub_shape(self):
        spec = importlib.util.find_spec("rusty_bacnet")
        assert spec is not None and spec.origin is not None
        origin = Path(spec.origin)
        candidates = [origin.with_suffix(".pyi"), origin.parent / "rusty_bacnet.pyi"]
        stub = next(path for path in candidates if path.exists())
        classes = {node.name: node for node in ast.parse(stub.read_text()).body
                   if isinstance(node, ast.ClassDef)}
        fields = {node.target.id: ast.unparse(node.annotation)
                  for node in classes["DccOutcomeCounters"].body
                  if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name)}
        self.assertEqual(fields, dict.fromkeys(FIELDS, "int"))
        method = next(node for node in classes["BACnetServer"].body
                      if isinstance(node, ast.AsyncFunctionDef)
                      and node.name == "dcc_outcome_counters")
        assert method.returns is not None
        self.assertEqual(ast.unparse(method.returns), "DccOutcomeCounters")
