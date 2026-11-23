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
    BACnetClient,
    BACnetServer,
    BipEndpoint,
    MstpEndpoint,
    ScEndpoint,
)

# The calls that take a RoutedTarget where they take an address (#2617, #2588).
ROUTED_CALLS = ("read_property", "read_property_multiple", "read_range", "read_log_page",
                "write_property", "atomic_read_file", "atomic_write_file",
                "subscribe_cov", "unsubscribe_cov", "subscribe_cov_property",
                "unsubscribe_cov_property", "subscribe_cov_property_multiple")


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


def stub_arg_kinds(
    method: ast.FunctionDef | ast.AsyncFunctionDef,
) -> list[tuple[str, inspect._ParameterKind]]:
    """Each stub parameter but ``self`` with the kind its position gives it."""
    kinds = [
        * [(arg.arg, inspect.Parameter.POSITIONAL_ONLY) for arg in method.args.posonlyargs],
        * [(arg.arg, inspect.Parameter.POSITIONAL_OR_KEYWORD) for arg in method.args.args],
        * [(arg.arg, inspect.Parameter.KEYWORD_ONLY) for arg in method.args.kwonlyargs],
    ]
    return [(name, kind) for name, kind in kinds if name != "self"]


def runtime_arg_kinds(callable_: object) -> list[tuple[str, inspect._ParameterKind]]:
    """Each runtime parameter but ``self`` with its kind."""
    return [
        (name, parameter.kind)
        for name, parameter in inspect.signature(callable_).parameters.items()
        if name != "self"
    ]


class EndpointStubParityTests(unittest.TestCase):
    def test_endpoint_classes_present_in_runtime_and_stub(self):
        tree = installed_stub()
        classes = stub_classes(tree)
        # The TypedDict EndpointStatus exists in the stub only, like
        # ScHubStatus.
        self.assertIn("EndpointStatus", classes)
        for name in (
            "BipEndpoint",
            "ScEndpoint",
            "MstpEndpoint",
        ):
            with self.subTest(symbol=name):
                self.assertIn(name, classes)
                self.assertTrue(hasattr(rusty_bacnet, name), f"runtime missing {name}")
        # The narrow role classes are gone: endpoint.client() is a
        # BACnetClient and endpoint.server() a BACnetServer (#2617).
        for gone in ("EndpointClient", "EndpointServer", "EndpointCov"):
            with self.subTest(gone=gone):
                self.assertNotIn(gone, classes)
                self.assertFalse(hasattr(rusty_bacnet, gone))

    def test_runtime_signatures_match_stub(self):
        tree = installed_stub()
        classes = stub_classes(tree)
        cases = [
            (BipEndpoint, ["device_instance", "device_name", "vendor_id", "interface",
                           "port", "broadcast_address", "network_number",
                           "network_port_instance", "max_apdu", "segmentation",
                           "device_uuid", "queue_capacity",
                           "apdu_timeout_ms", "apdu_retries", "registered_network_port",
                           "apdu_segment_timeout_ms",
                           "read_work_limit", "share_port_by_address", "share_wildcard_port",
                           "min_request_interval_ms", "bbmd", "foreign_device", "multicast",
                           "on_reinitialize", "after_reinitialize", "reinit_password",
                           "mutation_policy", "cov_policy", "time_sync_policy", "device_storage_path",
                           "source_audit_reporter",
                           "audit_recipient", "audit_device_bindings"]),
            (ScEndpoint, ["device_instance", "sc_hub", "sc_vmac", "sc_ca_cert",
                          "sc_client_cert", "sc_client_key", "sc_device_uuid",
                          "device_name", "vendor_id", "sc_heartbeat_interval_ms",
                          "sc_heartbeat_timeout_ms", "network_number",
                          "network_port_instance", "max_apdu", "segmentation",
                          "apdu_segment_timeout_ms", "queue_capacity", "read_work_limit",
                          "min_request_interval_ms", "on_reinitialize",
                          "after_reinitialize", "reinit_password",
                          "mutation_policy", "cov_policy", "time_sync_policy", "device_storage_path",
                          "source_audit_reporter",
                          "audit_recipient", "audit_device_bindings", "reconnect",
                          "sc_failover_hub", "sc_connect_timeout_ms"]),
            (MstpEndpoint, ["device_instance", "serial_port", "device_name", "vendor_id",
                            "mstp_baud", "mstp_mac", "mstp_max_master",
                            "mstp_max_info_frames", "max_apdu", "segmentation",
                            "device_uuid", "queue_capacity",
                            "apdu_timeout_ms", "apdu_retries", "apdu_segment_timeout_ms",
                            "read_work_limit",
                            "min_request_interval_ms", "mstp_extended_frames",
                            "mstp_echo", "mstp_echo_grace_ms",
                            "mstp_transmit_latency_pad_ms", "mstp_usage_timeout_ms",
                            "mstp_reply_timeout_ms", "on_reinitialize",
                            "after_reinitialize", "reinit_password",
                            "mutation_policy", "cov_policy", "time_sync_policy", "device_storage_path",
                            "source_audit_reporter",
                            "audit_recipient", "audit_device_bindings"]),
        ]
        for cls, expected in cases:
            with self.subTest(cls=cls.__name__):
                runtime = list(inspect.signature(cls).parameters)
                self.assertEqual(runtime, expected)
                method = stub_method(classes[cls.__name__], "__init__")
                self.assertEqual(stub_arg_names(method)[1:], expected)
                # Kinds too: a parameter that turns keyword-only (or stops
                # being so) on either side fails here.
                self.assertEqual(runtime_arg_kinds(cls), stub_arg_kinds(method))
                for keyword in ("read_work_limit", "min_request_interval_ms",
                                "mutation_policy", "cov_policy", "time_sync_policy"):
                    self.assertEqual(
                        inspect.signature(cls).parameters[keyword].kind,
                        inspect.Parameter.KEYWORD_ONLY,
                    )

    def test_role_and_owner_methods_match_stub(self):
        tree = installed_stub()
        classes = stub_classes(tree)
        # Native Future-returning owner methods.
        for cls, methods in (
            (BipEndpoint, ["start", "close", "client", "server", "local_address",
                           "status", "foreign_registration", "read_bdt", "read_fdt",
                           "broadcast_i_am"]),
            (ScEndpoint, ["start", "close", "client", "server", "local_address",
                          "status", "broadcast_i_am",
                          "connection_events", "connection_status"]),
            (MstpEndpoint, ["start", "close", "client", "server", "local_address",
                            "status", "broadcast_i_am"]),
        ):
            for name in methods:
                with self.subTest(cls=cls.__name__, method=name):
                    self.assertTrue(callable(getattr(cls, name)))
                    method = stub_method(classes[cls.__name__], name)
                    self.assertIsInstance(method, ast.FunctionDef)
                    self.assertIsInstance(method.returns, ast.Subscript)
                    self.assertEqual(ast.unparse(method.returns.value), "Awaitable")
                    if name == "client":
                        self.assertEqual(ast.unparse(method.returns), "Awaitable[BACnetClient]")
                    if name == "server":
                        self.assertEqual(ast.unparse(method.returns), "Awaitable[BACnetServer]")
        # Sync pending seam.
        for cls in (BipEndpoint, ScEndpoint, MstpEndpoint):
            for name in (
                "add_analog_input",
                "add_analog_value",
                "add_binary_input",
                "add_binary_value",
                "add_group",
            ):
                with self.subTest(cls=cls.__name__, method=name):
                    params = list(inspect.signature(getattr(cls, name)).parameters)
                    self.assertIn("instance", params)
                    self.assertIn("name", params)
                    method = stub_method(classes[cls.__name__], name)
                    self.assertEqual(
                        runtime_arg_kinds(getattr(cls, name)), stub_arg_kinds(method)
                    )
        # The object builders take the server builders' keywords (#1867),
        # and their event algorithm parameter keywords (#2410), keyword-only.
        value_keywords = ["present_value_access", "value_source_tracking", "audit_level",
                          "auditable_operations", "audit_priority_filter"]
        profile_keywords = ["tags", "profile_location", "profile_name"]
        delays = ["time_delay", "time_delay_normal"]
        analog = ["high_limit", "low_limit", "deadband", "limit_enable"] + delays
        binary = ["alarm_value"] + delays
        builders = {
            "add_analog_input": (["instance", "name", "units", "present_value"],
                                 profile_keywords + analog),
            "add_analog_output": (["instance", "name", "units"],
                                  ["present_value"] + profile_keywords + analog),
            "add_analog_value": (["instance", "name", "units"],
                                 ["present_value"] + value_keywords + profile_keywords + analog),
            "add_binary_input": (["instance", "name"],
                                 ["present_value", "polarity"] + profile_keywords + binary),
            "add_binary_output": (["instance", "name"],
                                  ["present_value", "polarity"] + profile_keywords + delays),
            "add_binary_value": (["instance", "name"],
                                 ["present_value"] + value_keywords + profile_keywords + binary),
            "add_multistate_input": (["instance", "name", "number_of_states"],
                                     ["present_value"] + profile_keywords + ["alarm_values"] + delays),
            "add_multistate_output": (["instance", "name", "number_of_states"],
                                      ["present_value"] + profile_keywords + delays),
            "add_multistate_value": (["instance", "name", "number_of_states"],
                                     ["present_value"] + value_keywords[:2] + profile_keywords
                                     + ["alarm_values"] + delays),
        }
        positional = inspect.Parameter.POSITIONAL_OR_KEYWORD
        keyword = inspect.Parameter.KEYWORD_ONLY
        for cls in (BipEndpoint, ScEndpoint, MstpEndpoint):
            for name, (leading, keywords) in builders.items():
                with self.subTest(cls=cls.__name__, builder=name):
                    expected = ([(arg, positional) for arg in leading]
                                + [(arg, keyword) for arg in keywords])
                    self.assertEqual(runtime_arg_kinds(getattr(cls, name)), expected)
                    method = stub_method(classes[cls.__name__], name)
                    self.assertEqual(stub_arg_kinds(method), expected)
        # Every endpoint selects a source Audit Reporter (#2348, #2357).
        expected = [("instance", positional), ("name", positional),
                    ("audit_level", keyword), ("auditable_operations", keyword),
                    ("issue_confirmed_notifications", keyword)]
        for cls in (BipEndpoint, ScEndpoint, MstpEndpoint):
            with self.subTest(cls=cls.__name__, method="add_audit_reporter"):
                self.assertEqual(runtime_arg_kinds(cls.add_audit_reporter), expected)
                reporter_stub = stub_method(classes[cls.__name__], "add_audit_reporter")
                self.assertEqual(stub_arg_kinds(reporter_stub), expected)
                # The level and operations are required, as on the server's
                # configure_audit_reporters: a default would silently record nothing.
                parameters = inspect.signature(cls.add_audit_reporter).parameters
                for name in ("audit_level", "auditable_operations"):
                    self.assertIs(parameters[name].default, inspect.Parameter.empty, name)
                stub_defaults = dict(zip((arg.arg for arg in reporter_stub.args.kwonlyargs),
                                         reporter_stub.args.kw_defaults))
                self.assertIsNone(stub_defaults["audit_level"])
                self.assertIsNone(stub_defaults["auditable_operations"])
                self.assertIsNotNone(stub_defaults["issue_confirmed_notifications"])
        # Roles: the client's routed calls take a str or a RoutedTarget.
        for name in ROUTED_CALLS:
            with self.subTest(method=name):
                node = stub_method(classes["BACnetClient"], name)
                self.assertEqual(ast.unparse(node.args.args[1].annotation), "str | RoutedTarget")
                self.assertEqual(runtime_arg_kinds(getattr(BACnetClient, name)),
                                 stub_arg_kinds(node))
        # The server role is the standalone class, whose runtime calls the
        # endpoint's server has.
        for name in ("read_property", "write_property_local", "set_present_value_local",
                     "set_reliability_local", "comm_state", "start", "stop"):
            with self.subTest(method=name):
                self.assertTrue(callable(getattr(BACnetServer, name)))
                stub_method(classes["BACnetServer"], name)
        # The local changes are the server's, not the endpoint's (#2617).
        for cls in (BipEndpoint, ScEndpoint, MstpEndpoint):
            for gone in ("set_present_value_local", "set_reliability_local"):
                with self.subTest(cls=cls.__name__, gone=gone):
                    self.assertFalse(hasattr(cls, gone))

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
                "class EndpointStatus",

                "class RoutedTarget",
            ):
                with self.subTest(symbol=symbol):
                    self.assertIn(symbol, repo_text)
                    self.assertIn(symbol, installed_text)


if __name__ == "__main__":
    unittest.main()
