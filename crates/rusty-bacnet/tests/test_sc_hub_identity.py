"""Hub-only constructor and installed-stub contract, without credential I/O."""
import ast
import inspect
import socket
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import ScHub


class HubIdentityTests(unittest.TestCase):
    def test_uuid_required_length_zero_and_vmac_errors_precede_io(self):
        with socket.socket() as occupied:
            occupied.bind(("127.0.0.1", 0))
            occupied.listen()
            args = (f"127.0.0.1:{occupied.getsockname()[1]}", "absent-cert.pem",
                    "absent-key.pem", b"\x02\0\0\0\0\1", "absent-ca.pem")
            for kwargs, message in [({}, "required"), ({"device_uuid": None}, "required"),
                                     *[({"device_uuid": bytes(n)}, "exactly 16") for n in (0, 15, 17)],
                                     ({"device_uuid": bytes(16)}, "all zero")]:
                with self.subTest(kwargs=kwargs), self.assertRaisesRegex(ValueError, message):
                    ScHub(*args, **kwargs)
            for vmac in (bytes(6), b"\xff" * 6):
                with self.subTest(vmac=vmac), self.assertRaisesRegex(ValueError, "vmac.*UNKNOWN.*BROADCAST"):
                    ScHub(*args[:3], vmac, args[4], device_uuid=b"\1" * 16)
            for vmac in (b"", bytes(5), bytes(7)):
                # Old VMAC-length exception and precedence over missing UUID.
                with self.subTest(vmac=vmac), self.assertRaisesRegex(RuntimeError, "vmac.*6 bytes"):
                    ScHub(*args[:3], vmac, args[4])
            for ca in (None, ""):
                with self.subTest(ca=ca), self.assertRaisesRegex(ValueError, "ca_cert"):
                    ScHub(*args[:3], b"", ca, device_uuid=bytes(16))
            # A valid constructor succeeds despite the occupied address and
            # nonexistent credentials: constructors must not read files or bind.
            for uuid in (b"\0" * 15 + b"\1", bytearray(b"\1" * 16)):
                ScHub(*args, device_uuid=uuid)
            with self.assertRaises(TypeError):
                ScHub(*args, b"\1" * 16)  # type: ignore[call-arg]  # No sixth positional slot.

    def test_installed_hub_stub_matches_runtime_keyword_contract(self):
        stub = Path(rusty_bacnet.__file__).with_name("__init__.pyi")
        tree = ast.parse(stub.read_text())
        cls = next(node for node in tree.body if isinstance(node, ast.ClassDef) and node.name == "ScHub")
        init = next(node for node in cls.body if isinstance(node, ast.FunctionDef) and node.name == "__init__")
        self.assertEqual([arg.arg for arg in init.args.args],
                         ["self", "listen", "cert", "key", "vmac", "ca_cert"])
        kwonly = ["device_uuid", "max_clients", "max_handshakes", "admission_policy",
                  "graceful_disconnect_ack_ms", "graceful_ws_close_ms", "graceful_overall_ms",
                  "handshake_tls_ms", "handshake_websocket_upgrade_ms",
                  "handshake_connect_request_ms",
                  'probe_scan_interval_ms', 'probe_idle_age_ms', 'probe_ack_age_ms', 'probe_send_budget_ms', 'broadcast_sender_burst', 'broadcast_sender_per_second', 'broadcast_global_burst', 'broadcast_global_per_second', 'relay_send_budget_ms']
        self.assertEqual([arg.arg for arg in init.args.kwonlyargs], kwonly)
        defaults = {arg.arg: default for arg, default in
                    zip(init.args.kwonlyargs, init.args.kw_defaults)}
        for name in kwonly:
            assert defaults[name] is not None, f"stub default missing for {name}"
        self.assertIsNone(ast.literal_eval(defaults["device_uuid"]))  # type: ignore[arg-type]
        self.assertEqual(
            {name: ast.literal_eval(defaults[name]) for name in kwonly[1:]},  # type: ignore[arg-type]
            {"max_clients": 256, "max_handshakes": 256, "admission_policy": "allow_all",
             "graceful_disconnect_ack_ms": 5000, "graceful_ws_close_ms": 5000,
             "graceful_overall_ms": 15000, "handshake_tls_ms": 10000,
             "handshake_websocket_upgrade_ms": 10000, "handshake_connect_request_ms": 10000,
             'probe_scan_interval_ms': 30000, 'probe_idle_age_ms': 60000, 'probe_ack_age_ms': 5000, 'probe_send_budget_ms': 5000, 'broadcast_sender_burst': 1024, 'broadcast_sender_per_second': 128, 'broadcast_global_burst': 4096, 'broadcast_global_per_second': 512, 'relay_send_budget_ms': 5000})
        uuid_type = init.args.kwonlyargs[0].annotation
        assert uuid_type is not None
        annotation = ast.unparse(uuid_type)
        self.assertIn("bytes", annotation)
        self.assertIn("bytearray", annotation)
        params = inspect.signature(ScHub).parameters
        self.assertEqual(list(params), [arg.arg for arg in init.args.args[1:]] + kwonly)
        for name in kwonly:
            self.assertEqual(params[name].kind, inspect.Parameter.KEYWORD_ONLY)
            self.assertEqual(params[name].default, ast.literal_eval(defaults[name]))
        self.assertIsNone(params["device_uuid"].default)
        self.assertEqual(params["admission_policy"].default, "allow_all")
        # Lifecycle methods exist in both the stub and the runtime.
        # (async def parses as AsyncFunctionDef, not FunctionDef.)
        funcs = (ast.FunctionDef, ast.AsyncFunctionDef)
        stub_methods = {node.name for node in cls.body if isinstance(node, funcs)}
        for name in ("start", "stop", "shutdown_gracefully", "status",
                     "__aenter__", "__aexit__", "address", "url"):
            with self.subTest(method=name):
                self.assertIn(name, stub_methods)
                self.assertTrue(callable(getattr(ScHub, name, None)))
        # address()/url() return None before start: the stub must say Optional.
        for name in ("address", "url"):
            node = next(node for node in cls.body
                        if isinstance(node, funcs) and node.name == name)
            assert node.returns is not None
            self.assertIn("Optional", ast.unparse(node.returns))
        shutdown = next(node for node in cls.body
                        if isinstance(node, funcs) and node.name == "shutdown_gracefully")
        assert shutdown.returns is not None
        outcome = ast.unparse(shutdown.returns)
        self.assertIn("graceful", outcome)
        self.assertIn("forced", outcome)
