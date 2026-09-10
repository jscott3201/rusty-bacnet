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
        self.assertEqual([arg.arg for arg in init.args.kwonlyargs], ["device_uuid"])
        default = init.args.kw_defaults[0]
        assert default is not None
        self.assertIsNone(ast.literal_eval(default))
        uuid_type = init.args.kwonlyargs[0].annotation
        assert uuid_type is not None
        annotation = ast.unparse(uuid_type)
        self.assertIn("bytes", annotation)
        self.assertIn("bytearray", annotation)
        params = inspect.signature(ScHub).parameters
        self.assertEqual(list(params), [arg.arg for arg in init.args.args[1:]] + ["device_uuid"])
        self.assertEqual(params["device_uuid"].kind, inspect.Parameter.KEYWORD_ONLY)
        self.assertIsNone(params["device_uuid"].default)
