"""Installed-artifact tests for the typed Python Audit client boundary."""

from __future__ import annotations

import ast
import asyncio
import inspect
import tempfile
import unittest
from pathlib import Path
from types import MappingProxyType
from typing import Any, Callable, cast

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
            "successful_actions_only": True,
        },
        "requested_count": 10,
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
                    "successful_actions_only": 1,
                },
            },
            {
                **target_query(),
                "query_parameters": {
                    **target_query()["query_parameters"],
                    "operations": True,
                },
            },
        ]
        for request in type_errors:
            with self.subTest(type_error=request), self.assertRaises(TypeError):
                _unused = method(ADDRESS, request)

        value_errors = [
            {},
            {**target_query(), "extra": 1},
            {**target_query(), "requested_count": -1},
            {**target_query(), "requested_count": 65_536},
            {**target_query(), "start_at_sequence_number": -1},
            {**target_query(), "start_at_sequence_number": 2**32},
            {**target_query(), "query_parameters": {"kind": "by_target"}},
            {
                **target_query(),
                "query_parameters": {
                    **target_query()["query_parameters"],
                    "kind": "target",
                },
            },
        ]
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
                    "successful_actions_only": False,
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

    async def _exercise_live_server(self) -> None:
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

                    invalid = notification_request(
                        minimal_notification(AuditOperation.from_raw(16))
                    )
                    with self.assertRaises(ValueError):
                        _unused = typed_unconfirmed(address, invalid)

                    notification = notification_request(
                        {
                            **minimal_notification(AuditOperation.WRITE),
                            "source_timestamp": BACnetTimeStamp.sequence_number(1),
                            "target_device": address_recipient(),
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
            finally:
                await server.stop()


if __name__ == "__main__":
    unittest.main()
