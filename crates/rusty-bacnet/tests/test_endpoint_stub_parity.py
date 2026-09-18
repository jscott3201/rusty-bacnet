"""AST/signature parity for RB-19 endpoint bindings (installed extension).

Asserts the installed .pyi matches runtime signatures for every new public
symbol, and that the repo stub copy matches the installed copy (installed-
release vs dev-build consistency per repo convention).
"""

from __future__ import annotations

import ast
import inspect
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import (
    BipEndpoint,
    EndpointClient,
    EndpointServer,
    MstpEndpoint,
    ScEndpoint,
)


def installed_stub() -> ast.Module:
    stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    return ast.parse(stub_path.read_text(encoding="utf-8"), filename=str(stub_path))


def stub_classes(tree: ast.Module) -> dict[str, ast.ClassDef]:
    return {node.name: node for node in tree.body if isinstance(node, ast.ClassDef)}


def stub_method(class_node: ast.ClassDef, name: str) -> ast.FunctionDef | ast.AsyncFunctionDef:
    for node in class_node.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name == name:
            return node
    raise AssertionError(f"{class_node.name}.{name} missing from stub")


def stub_arg_names(method: ast.FunctionDef | ast.AsyncFunctionDef) -> list[str]:
    return [
        * [arg.arg for arg in method.args.posonlyargs],
        * [arg.arg for arg in method.args.args],
        * [arg.arg for arg in method.args.kwonlyargs],
    ]


class EndpointStubParityTests(unittest.TestCase):
    def test_endpoint_classes_present_in_runtime_and_stub(self):
        tree = installed_stub()
        classes = stub_classes(tree)
        # TypedDicts (EndpointStatus) exist in the stub only, like ScHubStatus.
        for name in ("EndpointStatus",):
            with self.subTest(symbol=name):
                self.assertIn(name, classes)
        for name in (
            "EndpointClient",
            "EndpointServer",
            "BipEndpoint",
            "ScEndpoint",
            "MstpEndpoint",
        ):
            with self.subTest(symbol=name):
                self.assertIn(name, classes)
                self.assertTrue(hasattr(rusty_bacnet, name), f"runtime missing {name}")

    def test_runtime_signatures_match_stub(self):
        tree = installed_stub()
        classes = stub_classes(tree)
        cases = [
            (BipEndpoint, ["device_instance", "device_name", "vendor_id", "interface",
                           "port", "broadcast_address", "network_number",
                           "network_port_instance", "max_apdu", "segmentation",
                           "services", "device_uuid", "queue_capacity",
                           "apdu_timeout_ms", "apdu_retries"]),
            (ScEndpoint, ["device_instance", "sc_hub", "sc_vmac", "sc_ca_cert",
                          "sc_client_cert", "sc_client_key", "sc_device_uuid",
                          "device_name", "vendor_id", "sc_heartbeat_interval_ms",
                          "sc_heartbeat_timeout_ms", "network_number",
                          "network_port_instance", "max_apdu", "segmentation",
                          "services", "queue_capacity"]),
            (MstpEndpoint, ["device_instance", "serial_port", "device_name", "vendor_id",
                            "mstp_baud", "mstp_mac", "mstp_max_master",
                            "mstp_max_info_frames", "max_apdu", "segmentation",
                            "services", "device_uuid", "queue_capacity",
                            "apdu_timeout_ms", "apdu_retries"]),
        ]
        for cls, expected in cases:
            with self.subTest(cls=cls.__name__):
                runtime = list(inspect.signature(cls).parameters)
                self.assertEqual(runtime, expected)
                method = stub_method(classes[cls.__name__], "__init__")
                self.assertEqual(stub_arg_names(method)[1:], expected)

    def test_role_and_owner_methods_match_stub(self):
        tree = installed_stub()
        classes = stub_classes(tree)
        # Async owner methods.
        for cls, methods in (
            (BipEndpoint, ["start", "close", "client", "server", "local_address",
                           "status", "broadcast_i_am"]),
            (ScEndpoint, ["start", "close", "client", "server", "local_address",
                          "status", "broadcast_i_am"]),
            (MstpEndpoint, ["start", "close", "client", "server", "local_address",
                            "status", "broadcast_i_am"]),
        ):
            for name in methods:
                with self.subTest(cls=cls.__name__, method=name):
                    self.assertTrue(callable(getattr(cls, name)))
                    stub_method(classes[cls.__name__], name)
                    self.assertTrue(
                        isinstance(
                            stub_method(classes[cls.__name__], name),
                            ast.AsyncFunctionDef,
                        ),
                        f"{cls.__name__}.{name} must be async in stub",
                    )
        # Sync pending seam.
        for cls in (BipEndpoint, ScEndpoint, MstpEndpoint):
            for name in (
                "add_analog_input",
                "add_analog_value",
                "add_binary_input",
                "add_binary_value",
            ):
                with self.subTest(cls=cls.__name__, method=name):
                    params = list(inspect.signature(getattr(cls, name)).parameters)
                    self.assertIn("instance", params)
                    self.assertIn("name", params)
                    stub_method(classes[cls.__name__], name)
        # Roles.
        self.assertEqual(
            list(inspect.signature(EndpointClient.read_property).parameters),
            ["self", "address", "object_id", "property_id", "array_index"],
        )
        stub_method(classes["EndpointClient"], "read_property")
        self.assertTrue(hasattr(EndpointServer, "is_session_alive"))
        stub_method(classes["EndpointServer"], "is_session_alive")
        stub_method(classes["EndpointServer"], "suspend_next_reply")

    def test_repo_and_installed_stubs_agree(self):
        repo_stub = (
            Path(__file__).parent.parent / "rusty_bacnet.pyi"
        )
        installed_stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
        # Dev-build consistency: `maturin develop` installs the repo stub.
        # Installed releases carry their own copy; this pins the two together
        # for the endpoint surface at least.
        if repo_stub.is_file():
            repo_text = repo_stub.read_text(encoding="utf-8")
            installed_text = installed_stub_path.read_text(encoding="utf-8")
            for symbol in (
                "class BipEndpoint",
                "class ScEndpoint",
                "class MstpEndpoint",
                "class EndpointClient",
                "class EndpointServer",
                "class EndpointStatus",
            ):
                with self.subTest(symbol=symbol):
                    self.assertIn(symbol, repo_text)
                    self.assertIn(symbol, installed_text)


if __name__ == "__main__":
    unittest.main()
