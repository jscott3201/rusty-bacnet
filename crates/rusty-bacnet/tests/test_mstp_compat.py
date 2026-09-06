"""Artifact tests for the Python MS/TP compatibility surface."""

from __future__ import annotations

import ast
import inspect
import re
import tempfile
import unittest
import uuid
from pathlib import Path
from typing import Any

import rusty_bacnet
from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
)


CLIENT_POSITIONAL = [
    "interface",
    "port",
    "broadcast_address",
    "apdu_timeout_ms",
    "transport",
    "sc_hub",
    "sc_vmac",
    "sc_ca_cert",
    "sc_client_cert",
    "sc_client_key",
    "sc_heartbeat_interval_ms",
    "sc_heartbeat_timeout_ms",
    "ipv6_interface",
]
SERVER_POSITIONAL = [
    "device_instance",
    "device_name",
    "interface",
    "port",
    "broadcast_address",
    "transport",
    "sc_hub",
    "sc_vmac",
    "sc_ca_cert",
    "sc_client_cert",
    "sc_client_key",
    "sc_heartbeat_interval_ms",
    "sc_heartbeat_timeout_ms",
    "ipv6_interface",
    "dcc_password",
    "reinit_password",
]
MSTP_KEYWORD_ONLY = [
    "serial_port",
    "mstp_baud",
    "mstp_mac",
    "mstp_max_master",
    "mstp_max_info_frames",
]
SUPPORTED_BAUD_RATES = (9_600, 19_200, 38_400, 57_600, 76_800, 115_200)
SUPPORTED_BAUD_ERROR = (
    "mstp_baud must be one of 9600, 19200, 38400, 57600, 76800, or 115200"
)


def nonexistent_serial_path() -> str:
    return str(
        Path(tempfile.gettempdir())
        / f"rusty-bacnet-missing-{uuid.uuid4()}"
        / "serial"
    )


def make_server(path: str, **overrides: Any) -> BACnetServer:
    config: dict[str, Any] = {
        "device_instance": 123001,
        "transport": "mstp",
        "serial_port": path,
        "mstp_baud": 38_400,
        "mstp_mac": 1,
        "mstp_max_master": 127,
        "mstp_max_info_frames": 1,
    }
    config.update(overrides)
    return BACnetServer(**config)


def make_client(path: str, **overrides: Any) -> BACnetClient:
    config: dict[str, Any] = {
        "transport": "mstp",
        "serial_port": path,
        "mstp_baud": 38_400,
        "mstp_mac": 1,
        "mstp_max_master": 127,
        "mstp_max_info_frames": 1,
    }
    config.update(overrides)
    return BACnetClient(**config)


def stub_signature(class_name: str) -> tuple[list[str], list[str]]:
    stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    tree = ast.parse(stub_path.read_text(encoding="utf-8"), filename=str(stub_path))
    class_node = next(
        node
        for node in tree.body
        if isinstance(node, ast.ClassDef) and node.name == class_name
    )
    init = next(
        node
        for node in class_node.body
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        and node.name == "__init__"
    )
    positional = [
        arg.arg
        for arg in [*init.args.posonlyargs, *init.args.args]
        if arg.arg != "self"
    ]
    keyword_only = [arg.arg for arg in init.args.kwonlyargs]
    return positional, keyword_only


class SignatureCompatibilityTests(unittest.TestCase):
    def assert_signature(
        self, cls: type, positional_names: list[str], keyword_only_names: list[str]
    ) -> None:
        parameters = list(inspect.signature(cls).parameters.values())
        self.assertEqual(
            [
                p.name
                for p in parameters
                if p.kind is inspect.Parameter.POSITIONAL_OR_KEYWORD
            ],
            positional_names,
        )
        self.assertEqual(
            [p.name for p in parameters if p.kind is inspect.Parameter.KEYWORD_ONLY],
            keyword_only_names,
        )
        self.assertEqual(
            [p.default for p in parameters if p.name in MSTP_KEYWORD_ONLY],
            [None, 38_400, 1, 127, 1],
        )

    def test_runtime_and_stub_signatures_match_compatibility_contract(self) -> None:
        self.assert_signature(BACnetClient, CLIENT_POSITIONAL, MSTP_KEYWORD_ONLY)
        self.assert_signature(BACnetServer, SERVER_POSITIONAL, MSTP_KEYWORD_ONLY)
        self.assertEqual(
            stub_signature("BACnetClient"), (CLIENT_POSITIONAL, MSTP_KEYWORD_ONLY)
        )
        self.assertEqual(
            stub_signature("BACnetServer"), (SERVER_POSITIONAL, MSTP_KEYWORD_ONLY)
        )

    def test_old_positional_password_order_remains_accepted(self) -> None:
        BACnetServer(
            123001,
            "Positional Compatibility",
            "0.0.0.0",
            0xBAC0,
            "255.255.255.255",
            "bip",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "dcc-password",
            "reinit-password",
        )

    def test_new_mstp_parameters_reject_positional_use(self) -> None:
        client_args: list[Any] = [
            "0.0.0.0",
            0xBAC0,
            "255.255.255.255",
            6_000,
            "bip",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "unexpected-positional-serial-port",
        ]
        with self.assertRaises(TypeError):
            BACnetClient(*client_args)

    def test_mstp_construction_does_not_open_serial(self) -> None:
        path = nonexistent_serial_path()
        self.assertIsInstance(make_client(path), BACnetClient)
        self.assertIsInstance(make_server(path), BACnetServer)


class MstpRuntimeTests(unittest.IsolatedAsyncioTestCase):
    async def test_supported_baud_rates_reach_serial_open_for_client_and_server(
        self,
    ) -> None:
        path = nonexistent_serial_path()
        for baud in SUPPORTED_BAUD_RATES:
            client = make_client(path, mstp_baud=baud)
            with self.subTest(kind="client", baud=baud), self.assertRaises(
                RuntimeError
            ) as caught:
                await client.__aenter__()
            self.assertTrue(str(caught.exception).startswith("Serial open failed"))

            server = make_server(path, mstp_baud=baud)
            with self.subTest(kind="server", baud=baud), self.assertRaises(
                RuntimeError
            ) as caught:
                await server.start()
            self.assertTrue(str(caught.exception).startswith("Serial open failed"))

    async def test_unsupported_baud_rates_fail_before_io_for_client_and_server(
        self,
    ) -> None:
        path = nonexistent_serial_path()
        for baud in (0, 12_345):
            client = make_client(path, mstp_baud=baud)
            with self.subTest(kind="client", baud=baud), self.assertRaisesRegex(
                ValueError, f"^{re.escape(SUPPORTED_BAUD_ERROR)}$"
            ):
                await client.__aenter__()

            server = make_server(path, mstp_baud=baud)
            with self.subTest(kind="server", baud=baud), self.assertRaisesRegex(
                ValueError, f"^{re.escape(SUPPORTED_BAUD_ERROR)}$"
            ):
                await server.start()

    async def test_peer_boundaries_in_both_syntaxes_and_legacy_addresses(self) -> None:
        client = BACnetClient()
        oid = ObjectIdentifier(ObjectType.DEVICE, 1)
        for mac in (0, 127, 128, 254):
            for address in (str(mac), f"mstp:{mac}"):
                with self.subTest(address=address), self.assertRaisesRegex(
                    RuntimeError, "client not started"
                ):
                    await client.read_property(
                        address, oid, PropertyIdentifier.OBJECT_NAME
                    )
        for address in (
            "192.168.1.100:47808",
            "[::1]:47808",
            "01:02:03:04:05:06",
        ):
            with self.subTest(address=address), self.assertRaisesRegex(
                RuntimeError, "client not started"
            ):
                await client.read_property(address, oid, PropertyIdentifier.OBJECT_NAME)

    async def test_invalid_mstp_peers_have_explicit_value_errors(self) -> None:
        client = BACnetClient()
        oid = ObjectIdentifier(ObjectType.DEVICE, 1)
        cases = {
            "255": "MS/TP peer address 255 is broadcast, not a unicast peer",
            "mstp:255": "MS/TP peer address 255 is broadcast, not a unicast peer",
            "256": "MS/TP peer address must be in 0..=254",
            "mstp:256": "MS/TP peer address must be in 0..=254",
            "-1": "MS/TP peer address must be in 0..=254",
            "mstp:-1": "MS/TP peer address must be in 0..=254",
            "mstp:": "MS/TP peer address must be a decimal integer in 0..=254",
            "mstp:not-a-number": (
                "MS/TP peer address must be a decimal integer in 0..=254"
            ),
        }
        for address, message in cases.items():
            with self.subTest(address=address), self.assertRaisesRegex(
                ValueError, f"^{re.escape(message)}$"
            ):
                await client.read_property(address, oid, PropertyIdentifier.OBJECT_NAME)

    async def test_invalid_server_config_fails_before_open_and_preserves_registration(
        self,
    ) -> None:
        path = nonexistent_serial_path()
        cases = [
            (
                {"mstp_max_info_frames": 0},
                "mstp_max_info_frames must be in 1..=255",
            ),
            ({"mstp_mac": 128}, "mstp_mac must be in 0..=127"),
            ({"mstp_max_master": 128}, "mstp_max_master must be in 0..=127"),
            (
                {"mstp_mac": 4, "mstp_max_master": 3},
                "mstp_mac must be <= mstp_max_master",
            ),
        ]
        for index, (overrides, message) in enumerate(cases):
            server = make_server(path, **overrides)
            server.add_binary_value(index, f"Pending {index}")
            with self.subTest(overrides=overrides), self.assertRaisesRegex(
                ValueError, f"^{re.escape(message)}$"
            ):
                await server.start()
            # Failed pure validation must not mark the server started or block
            # further pending registration.
            server.add_binary_input(index, f"After failure {index}")

    async def test_client_uses_the_same_pure_validation(self) -> None:
        path = nonexistent_serial_path()
        for overrides, message in [
            (
                {"mstp_max_info_frames": 0},
                "mstp_max_info_frames must be in 1..=255",
            ),
        ]:
            client = make_client(path, **overrides)
            with self.subTest(overrides=overrides), self.assertRaisesRegex(
                ValueError, f"^{re.escape(message)}$"
            ):
                await client.__aenter__()

    async def test_serial_open_failure_keeps_server_retryable(self) -> None:
        server = make_server(nonexistent_serial_path())
        pending_count = getattr(server, "_pending_registration_count")
        server.add_binary_value(77, "Pending registration")
        self.assertEqual(pending_count(), 1)

        for attempt in range(2):
            with self.subTest(attempt=attempt), self.assertRaises(RuntimeError) as caught:
                await server.start()
            message = str(caught.exception)
            self.assertTrue(message.startswith("Serial open failed"), message)
            self.assertEqual(message.count("Serial open failed"), 1)
            self.assertEqual(pending_count(), 1)

        # A failed open must also leave the registration phase usable.
        server.add_binary_input(78, "Registered after retry")
        self.assertEqual(pending_count(), 2)


if __name__ == "__main__":
    unittest.main()
