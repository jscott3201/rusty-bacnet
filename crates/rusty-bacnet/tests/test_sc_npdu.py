"""Installed Python contract for the SC ingress quota and owned drop snapshots."""
import ast
import asyncio
import inspect
import os
from pathlib import Path
import subprocess
import sys
import unittest

import rusty_bacnet
from rusty_bacnet import BACnetClient, BACnetServer, ScReconnect
from sc_client_fixture import ClientScFixture
from test_endpoint_stub_parity import (
    installed_stub, runtime_arg_kinds, stub_arg_kinds, stub_classes, stub_method,
)


class ScNpduOptionsTests(unittest.TestCase):
    def test_keyword_only_defaults_bounds_and_pre_io_errors(self):
        for api, args in [(BACnetClient, ()), (BACnetServer, (3000,))]:
            with self.subTest(api=api.__name__):
                parameter = inspect.signature(api).parameters["sc_npdu_per_origin_limit"]
                self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
                self.assertIsNone(parameter.default)
                api(*args)
                api(*args, sc_npdu_per_origin_limit=None)
                for value in [0, 257]:
                    # No credentials, file lookup or network start can mask quota validation.
                    with self.assertRaisesRegex(ValueError, "per-origin limit"):
                        api(*args, transport="sc", sc_npdu_per_origin_limit=value)
                for value in [-1, 1 << 128]:
                    with self.assertRaises(OverflowError):
                        api(*args, transport="sc", sc_npdu_per_origin_limit=value)
                for value in [1.5, "1"]:
                    with self.assertRaises(TypeError):
                        api(*args, transport="sc", sc_npdu_per_origin_limit=value)
                for transport in ["bip", "ipv6", "mstp", "unknown"]:
                    with self.assertRaisesRegex(ValueError, "only.*sc"):
                        api(*args, transport=transport, sc_npdu_per_origin_limit=1)
                for value in [1, 32, 256]:
                    # Valid quota passes construction without reading nonexistent files.
                    api(*args, transport="sc", sc_npdu_per_origin_limit=value,
                        sc_ca_cert="/not/a/ca", sc_client_cert="/not/a/cert",
                        sc_client_key="/not/a/key", sc_device_uuid=b"a" * 16)

    def test_control_rate_limit_is_sc_only_and_checked_before_io(self):
        # #1969: (burst, per_second), validated with the native rules.
        for api, args in [(BACnetClient, ()), (BACnetServer, (3000,))]:
            with self.subTest(api=api.__name__):
                parameter = inspect.signature(api).parameters["sc_control_rate_limit"]
                self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
                self.assertIsNone(parameter.default)
                api(*args, sc_control_rate_limit=None)
                for value in [(0, 1), (1, 0), (4097, 1), (0, 0)]:
                    with self.assertRaisesRegex(ValueError, "control-message rate"):
                        api(*args, transport="sc", sc_control_rate_limit=value)
                for value in [(-1, 1), (1, 1 << 40)]:
                    with self.assertRaises(OverflowError):
                        api(*args, transport="sc", sc_control_rate_limit=value)
                for value in [5, ("1", 1)]:
                    with self.assertRaises(TypeError):
                        api(*args, transport="sc", sc_control_rate_limit=value)
                for value in [(1,), (1, 2, 3)]:
                    with self.assertRaisesRegex(ValueError, "length 2"):
                        api(*args, transport="sc", sc_control_rate_limit=value)
                for transport in ["bip", "ipv6", "mstp"]:
                    with self.assertRaisesRegex(ValueError, "sc_control_rate_limit.*only.*sc"):
                        api(*args, transport=transport, sc_control_rate_limit=(1, 1))
                for value in [(1, 1), (256, 32), (4096, 1 << 31)]:
                    api(*args, transport="sc", sc_control_rate_limit=value,
                        sc_ca_cert="/not/a/ca", sc_client_cert="/not/a/cert",
                        sc_client_key="/not/a/key", sc_device_uuid=b"a" * 16)

    def test_installed_stub_required_shape_and_future_signatures(self):
        path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
        ast.parse(path.read_text(), feature_version=(3, 11))
        classes = stub_classes(installed_stub())
        shape = classes["ScNpduDropCounts"]
        self.assertEqual([ast.unparse(base) for base in shape.bases], ["TypedDict"])
        self.assertEqual(shape.keywords, [])  # required keys, not total=False
        self.assertEqual({field.target.id: ast.unparse(field.annotation)
                          for field in shape.body if isinstance(field, ast.AnnAssign)},
                         dict(control_rate_drops="int", fairness_drops="int", full_drops="int",
                              closed_drops="int"))
        self.assertFalse(hasattr(rusty_bacnet, "ScNpduDropCounts"))
        for api in [BACnetClient, BACnetServer]:
            node = stub_method(classes[api.__name__], "sc_npdu_drop_counts")
            self.assertIsInstance(node, ast.FunctionDef)
            self.assertEqual(ast.unparse(node.returns), "Awaitable[ScNpduDropCounts]")
            self.assertEqual(runtime_arg_kinds(api.sc_npdu_drop_counts), stub_arg_kinds(node))
            method = stub_method(classes[api.__name__], "__init__")
            self.assertEqual(runtime_arg_kinds(api), stub_arg_kinds(method))
            arg = next(arg for arg in method.args.kwonlyargs
                       if arg.arg == "sc_npdu_per_origin_limit")
            self.assertEqual(ast.unparse(arg.annotation), "int | None")
            arg = next(arg for arg in method.args.kwonlyargs
                       if arg.arg == "sc_control_rate_limit")
            self.assertEqual(ast.unparse(arg.annotation), "tuple[int, int] | None")

    def test_real_tls_bursts_in_fresh_single_worker_runtime(self):
        # The native bridge initializes once per process. Set Tokio's worker
        # count before importing it in this child, independent of test ordering.
        result = subprocess.run(
            [sys.executable, str(Path(__file__).with_name("sc_npdu_wire.py")), "-v"],
            env={**os.environ, "TOKIO_WORKER_THREADS": "1"},
            capture_output=True, text=True, timeout=90)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


class ScControlRateWireTests(ClientScFixture):
    async def control_flood(self, options, count):
        """Hand the client `count` hub-relayed network-layer requests (message
        type 0x00) from one origin, then a heartbeat whose ack
        shows they all arrived; return the drop counts."""
        done = asyncio.Event()

        async def peer(reader, writer, connect):
            await self.accept_sc(writer, connect)
            origin = b"\x02\0\0\0\0\4"
            frames = b"".join(
                self.ws_frame(b"\x01\x08" + (i + 10).to_bytes(2, "big") + origin + b"\x01\x80\x00")
                for i in range(count))
            writer.write(frames + self.ws_frame(b"\x0a\0\x44\x55"))
            await asyncio.wait_for(writer.drain(), 3)
            for _ in range(40):
                _, value = await self.frame(reader, True)
                if value == b"\x0b\0\x44\x55":
                    break
            done.set()
            await asyncio.Event().wait()

        client = self.client(await self.raw_peer(peer), **options)
        await asyncio.wait_for(client.__aenter__(), 5)
        await asyncio.wait_for(done.wait(), 5)
        counts = await client.sc_npdu_drop_counts()
        await asyncio.wait_for(client.stop(), 5)
        return counts

    @staticmethod
    def ws_frame(payload):
        assert len(payload) < 126
        return bytes([0x82, len(payload)]) + payload

    async def test_a_small_limit_drops_and_counts_excess_control_messages(self):
        default = await self.control_flood({}, 12)
        self.assertEqual(default["control_rate_drops"], 0)
        # A burst of 2 refilling at 1 a second. The 12 frames go out in one
        # write and are processed promptly, so 10 are over budget, 9 if a
        # refill lands while they are read.
        limited = await self.control_flood({"sc_control_rate_limit": (2, 1)}, 12)
        self.assertGreaterEqual(limited["control_rate_drops"], 9, limited)
        self.assertLessEqual(limited["control_rate_drops"], 10, limited)
        self.assertEqual((limited["fairness_drops"], limited["full_drops"]), (0, 0))


class ScNpduLifecycleTests(ClientScFixture):
    async def test_non_sc_lifecycle_and_future_contract(self):
        for node in [BACnetClient("127.0.0.1", 0),
                     BACnetServer(3000, interface="127.0.0.1", port=0)]:
            self.addAsyncCleanup(self.stop_server, node)
            before = node.sc_npdu_drop_counts()
            self.assertIsInstance(before, asyncio.Future)
            self.assertFalse(inspect.iscoroutine(before))
            with self.assertRaisesRegex(RuntimeError, "not started"):
                await before
            await (node.start() if isinstance(node, BACnetServer) else node.__aenter__())
            with self.assertRaisesRegex(RuntimeError, "requires an SC"):
                await node.sc_npdu_drop_counts()
            await node.stop()
            with self.assertRaisesRegex(RuntimeError, "not started"):
                await node.sc_npdu_drop_counts()

    async def test_cancelled_snapshots_and_concurrent_stop_release_owners(self):
        hub, server = await self.serving_hub()
        client = self.client(await hub.url())
        await client.__aenter__()
        await self.read_value(client)
        for node in [client, server]:
            cancelled = asyncio.ensure_future(node.sc_npdu_drop_counts())
            cancelled.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await cancelled
            pending = [asyncio.ensure_future(node.sc_npdu_drop_counts()) for _ in range(32)]
            close = asyncio.ensure_future(node.stop())
            values = await asyncio.wait_for(
                asyncio.gather(*pending, close, return_exceptions=True), 5)
            self.assertIsNone(values[-1])
            for value in values[:-1]:
                if isinstance(value, Exception):
                    self.assertIsInstance(value, RuntimeError)
                    self.assertTrue("closing" in str(value) or "not started" in str(value))
                else:
                    self.assertEqual(value, dict(control_rate_drops=0, fairness_drops=0, full_drops=0,
                                                closed_drops=0))

    async def test_recovering_client_keeps_transport_snapshot_available(self):
        hub, _ = await self.serving_hub()
        relay = await self.relay(await hub.url())
        client = self.client(relay.url, reconnect=ScReconnect(
            initial_delay_ms=20, max_delay_ms=20, max_retries=10))
        await client.__aenter__()
        await relay.next_connection()
        before = await client.sc_npdu_drop_counts()
        relay.pause()
        await relay.disconnect()
        await relay.next_connection()  # retry socket is held before TLS/SC admission
        self.assertIsNone(await client.sc_peer_info())
        self.assertEqual(await client.sc_npdu_drop_counts(), before)
        await client.stop()
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await client.sc_npdu_drop_counts()


if __name__ == "__main__":
    unittest.main()
