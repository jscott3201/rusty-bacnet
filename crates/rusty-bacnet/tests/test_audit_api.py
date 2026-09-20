"""Installed-artifact tests for the typed Python Audit client and receiver."""

from __future__ import annotations

import ast
import asyncio
import inspect
import socket
import tempfile
import unittest
from pathlib import Path
from types import MappingProxyType
from typing import TYPE_CHECKING, Any, Callable, Literal, cast

import rusty_bacnet
from rusty_bacnet import (
    AuditOperation,
    BACnetClient,
    BACnetServer,
    BACnetTimeStamp,
    BacnetProtocolError,
    ErrorClass,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
)

if TYPE_CHECKING:
    from rusty_bacnet import AuditLogQueryRequestInput, AuditNotificationInput


ADDRESS = "127.0.0.1:47808"
STANDARD_OPERATIONS = (
    "READ",
    "WRITE",
    "CREATE",
    "DELETE",
    "LIFE_SAFETY",
    "ACKNOWLEDGE_ALARM",
    "DEVICE_DISABLE_COMM",
    "DEVICE_ENABLE_COMM",
    "DEVICE_RESET",
    "DEVICE_BACKUP",
    "DEVICE_RESTORE",
    "SUBSCRIPTION",
    "NOTIFICATION",
    "AUDITING_FAILURE",
    "NETWORK_CHANGES",
    "GENERAL",
)


def installed_stub() -> ast.Module:
    stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    return ast.parse(stub_path.read_text(encoding="utf-8"), filename=str(stub_path))


def stub_classes(tree: ast.Module) -> dict[str, ast.ClassDef]:
    return {
        node.name: node for node in tree.body if isinstance(node, ast.ClassDef)
    }


def stub_method(class_node: ast.ClassDef, name: str) -> ast.AsyncFunctionDef:
    return next(
        node
        for node in class_node.body
        if isinstance(node, ast.AsyncFunctionDef) and node.name == name
    )


def annotation_text(annotation: ast.expr | None) -> str:
    assert annotation is not None
    return ast.unparse(annotation)


def typed_dict_keys(class_node: ast.ClassDef) -> set[str]:
    return {
        node.target.id
        for node in class_node.body
        if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name)
    }


def device_recipient(instance: int) -> dict[str, Any]:
    return {
        "kind": "device",
        "object_identifier": ObjectIdentifier(ObjectType.DEVICE, instance),
    }


def address_recipient() -> dict[str, Any]:
    return {"kind": "address", "network_number": 65_535, "mac_address": b"\x01"}


def minimal_notification(operation: AuditOperation | None = None) -> dict[str, Any]:
    return {
        "source_device": device_recipient(1),
        "operation": operation or AuditOperation.READ,
        "target_device": device_recipient(2),
    }


def notification_request(notification: Any | None = None) -> dict[str, Any]:
    return {
        "notifications": [
            minimal_notification() if notification is None else notification
        ]
    }


def target_query() -> dict[str, Any]:
    return {
        "audit_log": ObjectIdentifier(ObjectType.AUDIT_LOG, 1),
        "query_parameters": {
            "kind": "by_target",
            "target_device_identifier": ObjectIdentifier(ObjectType.DEVICE, 2),
            # Corrected contract (RB-02/RB-20): 0 = all, 1 = successes-only,
            # 2 = failures-only. The pre-RB-02 Boolean is rejected (TypeError).
            "successful_actions_only": 1,
        },
        "requested_count": 10,
    }


def audit_snapshot(storage: str) -> dict[str, bytes]:
    """Compare both existing persistence slots without interpreting their schema."""
    return {
        suffix: path.read_bytes()
        for suffix in (".slot0", ".slot1")
        if (path := Path(storage + suffix)).exists()
    }


class AuditContractArtifactTests(unittest.TestCase):
    def test_enum_wrapper_constants_and_proprietary_boundaries(self) -> None:
        self.assertIs(rusty_bacnet.AuditOperation, AuditOperation)
        for raw, name in enumerate(STANDARD_OPERATIONS):
            with self.subTest(operation=name):
                value = getattr(AuditOperation, name)
                self.assertEqual(value.to_raw(), raw)
                self.assertEqual(value, AuditOperation.from_raw(raw))
                self.assertEqual(hash(value), raw)
        for raw in (32, 63):
            self.assertEqual(AuditOperation.from_raw(raw).to_raw(), raw)
        self.assertEqual(AuditOperation.from_raw(16).to_raw(), 16)
        self.assertEqual(AuditOperation.from_raw(64).to_raw(), 64)

    def test_runtime_stub_and_packaging_expose_exact_additive_contracts(self) -> None:
        module_path = Path(rusty_bacnet.__file__)
        self.assertTrue(module_path.with_suffix(".pyi").is_file())
        self.assertTrue(module_path.with_name("py.typed").is_file())

        tree = installed_stub()
        classes = stub_classes(tree)
        expected_typed_dicts = {
            "AuditRecipientDevice": {"kind", "object_identifier"},
            "AuditRecipientAddress": {"kind", "network_number", "mac_address"},
            "AuditPropertyReference": {
                "property_identifier",
                "property_array_index",
            },
            "AuditNotificationInput": {
                "source_timestamp",
                "target_timestamp",
                "source_device",
                "source_object",
                "operation",
                "source_comment",
                "target_comment",
                "invoke_id",
                "source_user_id",
                "source_user_role",
                "target_device",
                "target_object",
                "target_property",
                "target_priority",
                "target_value",
                "current_value",
                "result",
            },
            "AuditNotificationRequestInput": {"notifications"},
            "AuditLogQueryByTargetInput": {
                "kind",
                "target_device_identifier",
                "target_device_address",
                "target_object_identifier",
                "target_property_identifier",
                "target_array_index",
                "target_priority",
                "operations",
                "successful_actions_only",
            },
            "AuditLogQueryBySourceInput": {
                "kind",
                "source_device_identifier",
                "source_device_address",
                "source_object_identifier",
                "operations",
                "successful_actions_only",
            },
            "AuditLogQueryRequestInput": {
                "audit_log",
                "query_parameters",
                "start_at_sequence_number",
                "requested_count",
            },
            "AuditPropertyReferenceResult": {
                "property_identifier",
                "property_array_index",
            },
            "AuditNotification": {
                "source_timestamp",
                "target_timestamp",
                "source_device",
                "source_object",
                "operation",
                "source_comment",
                "target_comment",
                "invoke_id",
                "source_user_id",
                "source_user_role",
                "target_device",
                "target_object",
                "target_property",
                "target_priority",
                "target_value",
                "current_value",
                "result",
            },
            "AuditLogStatusDatum": {"kind", "log_status"},
            "AuditNotificationDatum": {"kind", "audit_notification"},
            "AuditTimeChangeDatum": {"kind", "time_change"},
            "AuditLogRecord": {"timestamp", "datum"},
            "AuditLogRecordResult": {"sequence_number", "record"},
            "AuditLogQueryAck": {"audit_log", "records", "no_more_items"},
        }
        for name, keys in expected_typed_dicts.items():
            with self.subTest(typed_dict=name):
                self.assertIn(name, classes)
                self.assertEqual(typed_dict_keys(classes[name]), keys)
                self.assertFalse(
                    hasattr(rusty_bacnet, name),
                    "TypedDict contracts must not create nominal runtime classes",
                )

        client = classes["BACnetClient"]
        typed_methods = {
            "confirmed_audit_notification_typed": (
                "AuditNotificationRequestInput",
                "None",
            ),
            "unconfirmed_audit_notification_typed": (
                "AuditNotificationRequestInput",
                "None",
            ),
            "audit_log_query_typed": ("AuditLogQueryRequestInput", "AuditLogQueryAck"),
        }
        for name, (request_type, return_type) in typed_methods.items():
            with self.subTest(method=name):
                runtime_parameters = list(
                    inspect.signature(getattr(BACnetClient, name)).parameters.values()
                )
                self.assertEqual(
                    [parameter.name for parameter in runtime_parameters],
                    ["self", "address", "request"],
                )
                self.assertTrue(
                    all(
                        parameter.default is inspect.Parameter.empty
                        for parameter in runtime_parameters
                    )
                )
                method = stub_method(client, name)
                arguments = [*method.args.posonlyargs, *method.args.args]
                self.assertEqual(
                    [argument.arg for argument in arguments],
                    ["self", "address", "request"],
                )
                self.assertEqual(annotation_text(arguments[1].annotation), "str")
                self.assertEqual(annotation_text(arguments[2].annotation), request_type)
                self.assertEqual(annotation_text(method.returns), return_type)

        name = "configure_audit_notification_sink"
        parameters = list(inspect.signature(getattr(BACnetServer, name)).parameters.values())
        self.assertEqual([p.name for p in parameters], ["self", "instance", "policy"])
        self.assertEqual(parameters[-1].kind, inspect.Parameter.KEYWORD_ONLY)
        self.assertIs(parameters[-1].default, inspect.Parameter.empty)
        method = next(
            node for node in classes["BACnetServer"].body
            if isinstance(node, ast.FunctionDef) and node.name == name
        )
        self.assertEqual(annotation_text(method.args.args[1].annotation), "int")
        self.assertEqual([arg.arg for arg in method.args.kwonlyargs], ["policy"])
        self.assertEqual(
            annotation_text(method.args.kwonlyargs[0].annotation),
            "Literal['deny_all', 'allow_all']",
        )
        self.assertEqual(annotation_text(method.returns), "None")

        raw_methods = {
            "confirmed_audit_notification": "None",
            "unconfirmed_audit_notification": "None",
            "audit_log_query": "bytes",
        }
        for name, return_type in raw_methods.items():
            with self.subTest(raw=name):
                self.assertEqual(
                    list(inspect.signature(getattr(BACnetClient, name)).parameters),
                    ["self", "address", "service_data"],
                )
                method = stub_method(client, name)
                arguments = [*method.args.posonlyargs, *method.args.args]
                self.assertEqual(
                    [argument.arg for argument in arguments],
                    ["self", "address", "service_data"],
                )
                self.assertEqual(annotation_text(arguments[2].annotation), "bytes")
                self.assertEqual(annotation_text(method.returns), return_type)

    def test_notification_validation_uses_documented_exception_classes(self) -> None:
        client = BACnetClient()
        method: Callable[[str, Any], Any] = client.confirmed_audit_notification_typed

        type_errors: list[Any] = [
            [],
            {"notifications": ()},
            {"notifications": [1]},
            notification_request(
                {
                    **minimal_notification(),
                    "operation": 0,
                }
            ),
            notification_request(
                {
                    **minimal_notification(),
                    "invoke_id": True,
                }
            ),
            notification_request(
                {
                    **minimal_notification(),
                    "target_value": bytearray(b"\x00"),
                }
            ),
            notification_request(
                {
                    **minimal_notification(),
                    "result": [ErrorClass.PROPERTY, ErrorCode.OTHER],
                }
            ),
        ]
        for request in type_errors:
            with self.subTest(type_error=request), self.assertRaises(TypeError):
                _unused = method(ADDRESS, request)

        value_errors: list[Any] = [
            {},
            {"notifications": []},
            {"notifications": [minimal_notification()], "extra": 1},
            notification_request({"operation": AuditOperation.READ}),
            notification_request(
                {
                    **minimal_notification(),
                    "source_device": {
                        "kind": "DEVICE",
                        "object_identifier": ObjectIdentifier(ObjectType.DEVICE, 1),
                    },
                }
            ),
        ]
        for raw in (16, 31, 64, 2**32 - 1):
            value_errors.append(notification_request(minimal_notification(AuditOperation.from_raw(raw))))
        for key, invalid in (
            ("invoke_id", -1),
            ("invoke_id", 256),
            ("source_user_id", 65_536),
            ("source_user_role", 256),
            ("target_priority", 0),
            ("target_priority", 17),
        ):
            value_errors.append(
                notification_request({**minimal_notification(), key: invalid})
            )
        value_errors.append(
            notification_request(
                {
                    **minimal_notification(),
                    "source_device": {
                        "kind": "address",
                        "network_number": 65_536,
                        "mac_address": b"",
                    },
                }
            )
        )
        value_errors.append(
            notification_request(
                {
                    **minimal_notification(),
                    "target_property": {
                        "property_identifier": PropertyIdentifier.PRESENT_VALUE,
                        "property_array_index": 2**64,
                    },
                }
            )
        )
        value_errors.append({"notifications": [minimal_notification()] * 10_001})
        for request in value_errors:
            with self.subTest(value_error=request), self.assertRaises(ValueError):
                _unused = method(ADDRESS, request)

    def test_query_validation_accepts_both_choices_and_rejects_invalid_values(self) -> None:
        client = BACnetClient()
        method: Callable[[str, Any], Any] = client.audit_log_query_typed

        type_errors = [
            [],
            {**target_query(), "requested_count": True},
            {
                **target_query(),
                "query_parameters": {
                    **target_query()["query_parameters"],
                    "successful_actions_only": True,
                },
            },
            {
                **target_query(),
                "query_parameters": {
                    **target_query()["query_parameters"],
                    "successful_actions_only": False,
                },
            },
            {
                **target_query(),
                "query_parameters": {
                    **target_query()["query_parameters"],
                    "operations": True,
                },
            },
            {
                **target_query(),
                "start_at_sequence_number": True,
            },
        ]
        for request in type_errors:
            with self.subTest(type_error=request), self.assertRaises(TypeError):
                _unused = method(ADDRESS, request)

        # The deprecated Boolean meaning stays a TypeError with guidance toward
        # the integer contract (1 for True, 0 for False).
        legacy_bool: Any = {
            **target_query(),
            "query_parameters": {
                **target_query()["query_parameters"],
                "successful_actions_only": True,
            },
        }
        with self.assertRaisesRegex(TypeError, "deprecated Boolean"):
            _unused = method(ADDRESS, legacy_bool)

        value_errors = [
            {},
            {**target_query(), "extra": 1},
            {**target_query(), "requested_count": -1},
            {**target_query(), "requested_count": 65_536},
            {**target_query(), "start_at_sequence_number": -1},
            # Corrected Unsigned64 cursor (RB-20): u32-range values are valid;
            # only the u64 domain edges are rejected.
            {**target_query(), "start_at_sequence_number": 2**64},
            {**target_query(), "query_parameters": {"kind": "by_target"}},
            {
                **target_query(),
                "query_parameters": {
                    **target_query()["query_parameters"],
                    "kind": "target",
                },
            },
        ]
        for invalid_filter in (-1, 3, 2**32):
            value_errors.append(
                {
                    **target_query(),
                    "query_parameters": {
                        **target_query()["query_parameters"],
                        "successful_actions_only": invalid_filter,
                    },
                }
            )
        for operations in (-1, 2**64, 1 << 16, 1 << 31):
            value_errors.append(
                {
                    **target_query(),
                    "query_parameters": {
                        **target_query()["query_parameters"],
                        "operations": operations,
                    },
                }
            )
        for request in value_errors:
            with self.subTest(value_error=request), self.assertRaises(ValueError):
                _unused = method(ADDRESS, request)

        async def accepted_before_lifecycle_check() -> None:
            by_source = {
                "audit_log": ObjectIdentifier(ObjectType.AUDIT_LOG, 1),
                "query_parameters": {
                    "kind": "by_source",
                    "source_device_identifier": ObjectIdentifier(ObjectType.DEVICE, 1),
                    "source_device_address": MappingProxyType(address_recipient()),
                    "source_object_identifier": None,
                    "operations": (1 << 0) | (1 << 15) | (1 << 32) | (1 << 63),
                    "successful_actions_only": 0,
                },
                "start_at_sequence_number": None,
                "requested_count": 65_535,
            }
            typed_query = cast(Callable[[str, Any], Any], client.audit_log_query_typed)
            with self.assertRaisesRegex(RuntimeError, "client not started"):
                await typed_query(ADDRESS, MappingProxyType(by_source))

        asyncio.run(accepted_before_lifecycle_check())

    def test_live_server_typed_and_raw_paths_preserve_fail_closed_behavior(self) -> None:
        asyncio.run(self._exercise_live_server())

    def test_explicit_deny_preserves_fail_closed_behavior(self) -> None:
        asyncio.run(self._exercise_live_server("deny_all"))

    def test_receiver_configuration_validation_preserves_registrations(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            server = BACnetServer(device_instance=503_513, interface="127.0.0.1", port=0)
            configure = cast(Callable[..., None], server.configure_audit_notification_sink)
            pending = getattr(server, "_pending_registration_count")
            with self.assertRaisesRegex(ValueError, "no pending Audit Log"):
                configure(1, policy="allow_all")
            server.add_analog_value(1, "Not an Audit Log")
            with self.assertRaisesRegex(ValueError, "no pending Audit Log"):
                configure(1, policy="allow_all")
            server.add_audit_log(1, "Sink", str(Path(directory) / "sink"))
            configure(1, policy="deny_all")
            for instance in (-1, 2**22, 2**100, True, 1.0, "1", None):
                with self.subTest(instance=instance), self.assertRaises(ValueError):
                    configure(instance, policy="allow_all")
            for policy in ("", "ALLOW_ALL", "allow", None, True, lambda _: True):
                with self.subTest(policy=policy), self.assertRaises(ValueError):
                    configure(1, policy=policy)
            self.assertEqual(pending(), 2)

            # Registration after selection must not silently replace the sink.
            server.add_audit_log(1, "Duplicate", str(Path(directory) / "duplicate"))
            for policy in ("deny_all", "allow_all"):
                with self.assertRaisesRegex(ValueError, "duplicate pending Audit Log"):
                    configure(1, policy=policy)
            for _ in range(2):
                # Synchronous failure even outside an event loop: no future,
                # transport or ownership transfer has been started.
                with self.assertRaisesRegex(ValueError, "duplicate pending Audit Log"):
                    _unused = server.start()
                self.assertEqual(pending(), 3)

            # Repair by explicitly selecting a different unambiguous log. Other
            # objects keep their existing same-ID replacement semantics.
            server.add_audit_log(2, "Repaired", str(Path(directory) / "repaired"))
            configure(2, policy="allow_all")

            async def retry() -> None:
                await server.start()
                try:
                    value = await server.read_property(
                        ObjectIdentifier(ObjectType.ANALOG_VALUE, 1),
                        PropertyIdentifier.OBJECT_NAME,
                    )
                    self.assertEqual(value.value, "Not an Audit Log")
                finally:
                    await server.stop()

            asyncio.run(retry())

    def test_static_receiver_commits_confirmed_and_unconfirmed_batches(self) -> None:
        asyncio.run(self._exercise_static_receiver())

    async def _exercise_static_receiver(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            storage = str(Path(directory) / "receiver")

            def receiver() -> BACnetServer:
                server = BACnetServer(
                    device_instance=503_512,
                    interface="127.0.0.1",
                    port=0,
                    broadcast_address="127.0.0.1",
                )
                server.add_audit_log(1, "Receiver", storage, buffer_size=10)
                # A second log must never become the sink by iteration order.
                server.add_audit_log(2, "Other", str(Path(directory) / "other"))
                server.configure_audit_notification_sink(2, policy="deny_all")
                server.configure_audit_notification_sink(1, policy="allow_all")
                with self.assertRaises(ValueError):
                    server.configure_audit_notification_sink(99, policy="deny_all")
                return server

            server = receiver()
            await server.start()
            try:
                with self.assertRaises(RuntimeError):
                    server.configure_audit_notification_sink(2, policy="deny_all")
                address = await server.local_address()
                async with BACnetClient(
                    interface="127.0.0.1", port=0,
                    broadcast_address="127.0.0.1", apdu_timeout_ms=2_000,
                ) as client:
                    confirmed = cast("AuditNotificationInput", {
                        **minimal_notification(AuditOperation.WRITE),
                        "source_timestamp": BACnetTimeStamp.sequence_number(17),
                        "source_comment": "confirmed",
                        "target_property": {
                            "property_identifier": PropertyIdentifier.PRESENT_VALUE,
                            "property_array_index": 2**64 - 1,
                        },
                        # Application Unsigned containing a value above 2**53.
                        "target_value": b"\x25\x08\xff\xff\xff\xff\xff\xff\xff\xff",
                        "current_value": None,
                        "result": None,
                    })
                    unconfirmed = cast("AuditNotificationInput", {
                        **minimal_notification(AuditOperation.READ),
                        "source_comment": "unconfirmed",
                        "target_timestamp": BACnetTimeStamp.sequence_number(18),
                        "result": (ErrorClass.PROPERTY, ErrorCode.OTHER),
                    })
                    self.assertIsNone(await client.confirmed_audit_notification_typed(
                        address, {"notifications": [confirmed, cast("AuditNotificationInput", {
                            **minimal_notification(), "source_comment": "batch item 2",
                        })]},
                    ))
                    query = cast("AuditLogQueryRequestInput", target_query())
                    query["query_parameters"]["successful_actions_only"] = 0
                    # A confirmed ACK already means both records committed.
                    self.assertEqual(len((await client.audit_log_query_typed(address, query))["records"]), 2)
                    self.assertIsNone(await client.unconfirmed_audit_notification_typed(
                        address, {"notifications": [unconfirmed, cast("AuditNotificationInput", {
                            **minimal_notification(), "source_comment": "batch item 4",
                        })]},
                    ))
                    # Unconfirmed completion means sent, not stored: poll the query
                    # under a deadline rather than assuming dispatch task ordering.
                    async with asyncio.timeout(3):
                        while True:
                            ack = await client.audit_log_query_typed(address, query)
                            if len(ack["records"]) == 4:
                                break
                            await asyncio.sleep(0.01)
                    self.assertIs(ack["no_more_items"], True)
                    records = ack["records"]
                    self.assertEqual([r["sequence_number"] for r in records], [4, 3, 2, 1])
                    datum = records[-1]["record"]["datum"]
                    assert datum["kind"] == "audit_notification"
                    saved = datum["audit_notification"]
                    assert saved["target_property"] is not None
                    assert saved["source_timestamp"] is not None
                    self.assertEqual(saved["target_property"]["property_array_index"], 2**64 - 1)
                    self.assertEqual(saved["target_value"], confirmed.get("target_value"))
                    self.assertEqual(saved["source_timestamp"].value, 17)
                    self.assertEqual(saved["source_device"], confirmed["source_device"])
                    self.assertEqual(saved["target_device"], confirmed["target_device"])
                    self.assertIsNone(saved["current_value"])
                    self.assertIsNone(saved["result"])
                    self.assertIsNone(saved["target_timestamp"])
                    datum = records[1]["record"]["datum"]
                    assert datum["kind"] == "audit_notification"
                    failed = datum["audit_notification"]
                    self.assertEqual(failed["result"], unconfirmed.get("result"))
                    self.assertIsNone(failed["source_timestamp"])
                    self.assertEqual((await client.audit_log_query_typed(address, {
                        **query, "audit_log": ObjectIdentifier(ObjectType.AUDIT_LOG, 2),
                    }))["records"], [])
            finally:
                await server.stop()

            # Reopen the actual file-backed object, not just the same in-memory DB.
            reopened = receiver()
            await reopened.start()
            try:
                async with BACnetClient(
                    interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                ) as client:
                    ack = await client.audit_log_query_typed(await reopened.local_address(), query)
                    self.assertEqual([r["sequence_number"] for r in ack["records"]], [4, 3, 2, 1])
                    datum = ack["records"][-1]["record"]["datum"]
                    assert datum["kind"] == "audit_notification"
                    saved = datum["audit_notification"]
                    assert saved["target_property"] is not None
                    self.assertEqual(saved["target_property"]["property_array_index"], 2**64 - 1)
                    self.assertEqual(saved["target_value"], confirmed.get("target_value"))
            finally:
                await reopened.stop()

    def test_receiver_wire_receipt_is_silent_after_reopen(self) -> None:
        asyncio.run(self._exercise_wire_receipt())

    async def _exercise_wire_receipt(self) -> None:
        # Ordinary loopback BACnet/IP fixtures: fixed outer Invoke ID permits
        # exact retransmission, which the high-level client intentionally hides.
        payload = bytes.fromhex("0e 2e 0c 02000001 2f 49 00 ae 0c 02000002 af 0f")
        confirmed = bytes.fromhex("81 0a 001c 01 04 00 05 2a 20") + payload
        unconfirmed = bytes.fromhex("81 0a 001a 01 00 10 0c") + payload
        loop = asyncio.get_running_loop()
        with tempfile.TemporaryDirectory() as directory, socket.socket(
            socket.AF_INET, socket.SOCK_DGRAM
        ) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            storage = str(Path(directory) / "wire")

            def receiver(policy: Literal["allow_all", "deny_all"]) -> BACnetServer:
                server = BACnetServer(
                    device_instance=503_514, interface="127.0.0.1", port=0,
                    broadcast_address="127.0.0.1",
                )
                server.add_audit_log(1, "Wire Sink", storage)
                server.configure_audit_notification_sink(1, policy=policy)
                return server

            async def send(server: BACnetServer, frame: bytes) -> None:
                ip, port = (await server.local_address()).split(":")
                await loop.sock_sendto(peer, frame, (ip, int(port)))

            async def silence(server: BACnetServer, frame: bytes) -> None:
                await send(server, frame)
                with self.assertRaises(TimeoutError):
                    await asyncio.wait_for(loop.sock_recv(peer, 2048), 0.1)

            server = receiver("allow_all")
            await server.start()
            try:
                await send(server, confirmed)
                response = await asyncio.wait_for(loop.sock_recv(peer, 2048), 2)
                self.assertEqual(response, bytes.fromhex("81 0a 0009 01 00 20 2a 20"))
                committed = audit_snapshot(storage)
                self.assertTrue(committed)
                await silence(server, confirmed)
                self.assertEqual(audit_snapshot(storage), committed)
                await silence(server, unconfirmed)
            finally:
                await server.stop()

            # A fresh server/tracker still sees the durable receipt. Existing
            # duplicate-before-authorization semantics survive even deny_all.
            reopened = receiver("deny_all")
            await reopened.start()
            try:
                before = audit_snapshot(storage)
                await silence(reopened, confirmed)
                await silence(reopened, unconfirmed)
                self.assertEqual(audit_snapshot(storage), before)
                async with BACnetClient(
                    interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                ) as client:
                    query = cast("AuditLogQueryRequestInput", target_query())
                    ack = await client.audit_log_query_typed(await reopened.local_address(), query)
                    # The unconfirmed request has no receipt identity and these
                    # timestamp-free records are not eligible for merging.
                    self.assertEqual(len(ack["records"]), 2)
            finally:
                await reopened.stop()

    async def _exercise_live_server(self, policy: Literal["deny_all"] | None = None) -> None:
        with tempfile.TemporaryDirectory() as directory:
            server = BACnetServer(
                device_instance=503_511,
                device_name="Typed Audit Artifact Test",
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
            )
            server.add_audit_log(
                1,
                "Audit Log",
                str(Path(directory) / "audit-log"),
                buffer_size=10,
            )
            if policy is not None:
                server.configure_audit_notification_sink(1, policy=policy)
            snapshot = audit_snapshot(str(Path(directory) / "audit-log"))
            await server.start()
            try:
                address = await server.local_address()
                async with BACnetClient(
                    interface="127.0.0.1",
                    port=0,
                    broadcast_address="127.0.0.1",
                    apdu_timeout_ms=2_000,
                ) as client:
                    typed_query = cast(
                        Callable[[str, Any], Any], client.audit_log_query_typed
                    )
                    typed_confirmed = cast(
                        Callable[[str, Any], Any],
                        client.confirmed_audit_notification_typed,
                    )
                    typed_unconfirmed = cast(
                        Callable[[str, Any], Any],
                        client.unconfirmed_audit_notification_typed,
                    )
                    request = target_query()
                    parameters = request["query_parameters"]
                    before = (
                        tuple(request),
                        request["audit_log"],
                        request["requested_count"],
                        dict(parameters),
                    )
                    ack = await typed_query(address, MappingProxyType(request))
                    self.assertEqual(set(ack), {"audit_log", "records", "no_more_items"})
                    self.assertEqual(ack["audit_log"], request["audit_log"])
                    self.assertEqual(ack["records"], [])
                    self.assertIs(ack["no_more_items"], True)
                    self.assertEqual(
                        before,
                        (
                            tuple(request),
                            request["audit_log"],
                            request["requested_count"],
                            dict(parameters),
                        ),
                    )

                    # RB-20 Python end-to-end on an empty log: both choices ×
                    # all three filters return an honest empty page, and the
                    # widened ranges (u64 cursor, u16 count edges) are accepted.
                    for filter_value in (0, 1, 2):
                        for choice in (
                            {
                                "kind": "by_target",
                                "target_device_identifier": ObjectIdentifier(
                                    ObjectType.DEVICE, 2
                                ),
                                "successful_actions_only": filter_value,
                            },
                            {
                                "kind": "by_source",
                                "source_device_identifier": ObjectIdentifier(
                                    ObjectType.DEVICE, 1
                                ),
                                "successful_actions_only": filter_value,
                            },
                        ):
                            with self.subTest(filter=filter_value, kind=choice["kind"]):
                                choice_query = {
                                    "audit_log": ObjectIdentifier(ObjectType.AUDIT_LOG, 1),
                                    "query_parameters": choice,
                                    "requested_count": 10,
                                }
                                empty = await typed_query(address, choice_query)
                                self.assertEqual(empty["records"], [])
                                self.assertIs(empty["no_more_items"], True)
                    for count in (0, 1, 65_535):
                        with self.subTest(requested_count=count):
                            counted = await typed_query(
                                address, {**request, "requested_count": count}
                            )
                            self.assertEqual(counted["records"], [])
                            # No retained record can match, so even count=0 is
                            # exhaustion here (cf. the Rust count=0/later-match
                            # case, which reports more items).
                            self.assertIs(counted["no_more_items"], True)
                    for cursor in (0, 2**32, 2**64 - 1):
                        with self.subTest(cursor=cursor):
                            paged = await typed_query(
                                address,
                                {**request, "start_at_sequence_number": cursor},
                            )
                            self.assertEqual(paged["records"], [])
                            self.assertIs(paged["no_more_items"], True)

                    # Local conversion failure (no frame sent) versus a valid
                    # empty result: the former raises before transport, the
                    # latter returns an honest page.
                    with self.assertRaises(TypeError):
                        _unused = typed_query(
                            address, {**request, "requested_count": True}
                        )
                    with self.assertRaisesRegex(TypeError, "deprecated Boolean"):
                        _unused = typed_query(
                            address,
                            {
                                **request,
                                "query_parameters": {
                                    **parameters,
                                    "successful_actions_only": False,
                                },
                            },
                        )

                    # An unknown Audit Log instance is a protocol error, unlike
                    # an unknown filter value which is an honest empty page.
                    with self.assertRaises(BacnetProtocolError) as raised:
                        await typed_query(
                            address,
                            {
                                **request,
                                "audit_log": ObjectIdentifier(
                                    ObjectType.AUDIT_LOG, 99
                                ),
                            },
                        )
                    self.assertEqual(
                        raised.exception.error_class, ErrorClass.OBJECT.to_raw()
                    )
                    self.assertEqual(
                        raised.exception.error_code,
                        ErrorCode.UNKNOWN_OBJECT.to_raw(),
                    )

                    invalid = notification_request(
                        minimal_notification(AuditOperation.from_raw(16))
                    )
                    with self.assertRaises(ValueError):
                        _unused = typed_unconfirmed(address, invalid)

                    notification = notification_request(
                        {
                            **minimal_notification(AuditOperation.WRITE),
                            "source_timestamp": BACnetTimeStamp.sequence_number(1),
                            "target_property": {
                                "property_identifier": PropertyIdentifier.PRESENT_VALUE,
                                "property_array_index": None,
                            },
                            "target_value": None,
                            "result": None,
                        }
                    )
                    with self.assertRaises(BacnetProtocolError) as raised:
                        await typed_confirmed(address, notification)
                    self.assertEqual(
                        raised.exception.error_class, ErrorClass.SERVICES.to_raw()
                    )
                    self.assertEqual(
                        raised.exception.error_code,
                        ErrorCode.SERVICE_REQUEST_DENIED.to_raw(),
                    )
                    self.assertIsNone(
                        await typed_unconfirmed(address, notification)
                    )
                    self.assertEqual(
                        (await typed_query(address, request))["records"],
                        [],
                    )

                    raw_query = bytes(
                        [
                            0x0C,
                            0x0F,
                            0x40,
                            0x00,
                            0x01,
                            0x1E,
                            0x0E,
                            0x0C,
                            0x02,
                            0x00,
                            0x00,
                            0x02,
                            0x79,
                            0x01,
                            0x0F,
                            0x1F,
                            0x39,
                            0x05,
                        ]
                    )
                    self.assertEqual(
                        await client.audit_log_query(address, raw_query),
                        bytes([0x0C, 0x0F, 0x40, 0x00, 0x01, 0x1E, 0x1F, 0x29, 0x01]),
                    )
                    raw_notification = bytes(
                        [
                            0x0E,
                            0x2E,
                            0x0C,
                            0x02,
                            0x00,
                            0x00,
                            0x01,
                            0x2F,
                            0x49,
                            0x00,
                            0xAE,
                            0x0C,
                            0x02,
                            0x00,
                            0x00,
                            0x02,
                            0xAF,
                            0x0F,
                        ]
                    )
                    await client.unconfirmed_audit_notification(address, raw_notification)
                    # Give the no-response service time to dispatch, then check
                    # both durable slots, including receipt-only mutations.
                    await asyncio.sleep(0.05)
                    self.assertEqual((await typed_query(address, request))["records"], [])
                    self.assertEqual(
                        audit_snapshot(str(Path(directory) / "audit-log")), snapshot
                    )
            finally:
                await server.stop()


if __name__ == "__main__":
    unittest.main()
