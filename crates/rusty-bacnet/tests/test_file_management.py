"""Artifact tests for synchronous pre-start built-in File management."""

from __future__ import annotations

import ast
import asyncio
import inspect
import re
import unittest
from pathlib import Path
from typing import Any

import rusty_bacnet
from rusty_bacnet import (
    BACnetServer,
    BacnetProtocolError,
    ErrorClass,
    ErrorCode,
)


METHOD_PARAMETERS = {
    "set_file_access_method": ["self", "instance", "access_method"],
    "set_file_data": ["self", "instance", "data"],
    "get_file_data": ["self", "instance"],
    "set_file_records": ["self", "instance", "records"],
    "get_file_records": ["self", "instance"],
    "set_max_file_size": ["self", "instance", "max_octets"],
    "set_max_record_count": ["self", "instance", "max_records"],
}
METHOD_ANNOTATIONS = {
    "set_file_access_method": (["int", "str"], "None"),
    "set_file_data": (["int", "bytes"], "None"),
    "get_file_data": (["int"], "bytes"),
    "set_file_records": (["int", "list[bytes]"], "None"),
    "get_file_records": (["int"], "list[bytes]"),
    "set_max_file_size": (["int", "int"], "int"),
    "set_max_record_count": (["int", "int"], "int"),
}
INVALID_ACCESS_METHOD = "access_method must be 'stream' or 'record'"


def stub_methods() -> dict[str, ast.FunctionDef | ast.AsyncFunctionDef]:
    stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    tree = ast.parse(stub_path.read_text(encoding="utf-8"), filename=str(stub_path))
    server = next(
        node
        for node in tree.body
        if isinstance(node, ast.ClassDef) and node.name == "BACnetServer"
    )
    return {
        node.name: node
        for node in server.body
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
    }


def annotation_text(annotation: ast.expr | None) -> str:
    assert annotation is not None
    return ast.unparse(annotation)


def make_server(**overrides: Any) -> BACnetServer:
    config: dict[str, Any] = {
        "device_instance": 420_708,
        "device_name": "File Management Artifact Test",
        "port": 0,
    }
    config.update(overrides)
    return BACnetServer(**config)


class FileManagementSignatureTests(unittest.TestCase):
    def test_runtime_and_stub_signatures_are_exact_and_synchronous(self) -> None:
        methods = stub_methods()
        for name, expected in METHOD_PARAMETERS.items():
            with self.subTest(method=name):
                runtime = getattr(BACnetServer, name)
                parameters = list(inspect.signature(runtime).parameters.values())
                self.assertEqual([parameter.name for parameter in parameters], expected)
                self.assertIs(parameters[0].kind, inspect.Parameter.POSITIONAL_ONLY)
                self.assertTrue(
                    all(
                        parameter.kind is inspect.Parameter.POSITIONAL_OR_KEYWORD
                        for parameter in parameters[1:]
                    )
                )
                self.assertFalse(inspect.iscoroutinefunction(runtime))

                stub = methods[name]
                self.assertIs(type(stub), ast.FunctionDef)
                self.assertEqual([arg.arg for arg in stub.args.posonlyargs], ["self"])
                self.assertEqual([arg.arg for arg in stub.args.args], expected[1:])
                annotations, returns = METHOD_ANNOTATIONS[name]
                self.assertEqual(
                    [annotation_text(arg.annotation) for arg in stub.args.args],
                    annotations,
                )
                self.assertEqual(annotation_text(stub.returns), returns)


class FileManagementRuntimeTests(unittest.TestCase):
    def test_stream_and_record_payloads_are_copied(self) -> None:
        server = make_server()
        server.add_file(1, "Stream")
        source = b"stream payload"
        server.set_file_data(1, source)
        first = server.get_file_data(1)
        second = server.get_file_data(1)
        self.assertIsInstance(first, bytes)
        self.assertEqual(first, source)
        self.assertIsNot(first, second)
        mutable_copy = bytearray(first)
        mutable_copy[0] = ord("S")
        self.assertEqual(server.get_file_data(1), source)

        server.add_file(2, "Records")
        server.set_file_access_method(2, "record")
        source_records = [b"one", b"two"]
        server.set_file_records(2, source_records)
        source_records[0] = b"changed"
        source_records.append(b"three")

        records = server.get_file_records(2)
        second_records = server.get_file_records(2)
        self.assertEqual(records, [b"one", b"two"])
        self.assertIsNot(records, second_records)
        self.assertIsNot(records[0], second_records[0])
        records[0] = b"changed"
        records.pop()
        self.assertEqual(server.get_file_records(2), [b"one", b"two"])

    def test_errors_caps_and_mode_mismatches_are_atomic(self) -> None:
        server = make_server()
        server.add_file(3, "Stream")
        server.set_file_data(3, b"preloaded")

        with self.assertRaisesRegex(
            ValueError, f"^{re.escape(INVALID_ACCESS_METHOD)}$"
        ):
            server.set_file_access_method(3, "STREAM")
        self.assertEqual(server.get_file_data(3), b"preloaded")

        with self.assertRaises(BacnetProtocolError) as mismatch:
            server.get_file_records(3)
        self.assertEqual(mismatch.exception.error_class, ErrorClass.SERVICES.to_raw())
        self.assertEqual(
            mismatch.exception.error_code,
            ErrorCode.INVALID_FILE_ACCESS_METHOD.to_raw(),
        )
        self.assertEqual(server.get_file_data(3), b"preloaded")

        self.assertEqual(server.set_max_file_size(3, (1 << 64) - 1), (1 << 31) - 1)
        self.assertEqual(server.set_max_file_size(3, 2), 2)
        self.assertEqual(server.get_file_data(3), b"preloaded")
        with self.assertRaises(OverflowError):
            server.set_max_file_size(3, -1)
        with self.assertRaises(OverflowError):
            server.set_max_file_size(3, 1 << 64)
        self.assertEqual(server.get_file_data(3), b"preloaded")

        server.add_file(4, "Records")
        server.set_file_access_method(4, "record")
        server.set_file_records(4, [b"one", b"two"])
        invalid_records: Any = [b"valid", bytearray(b"not bytes")]
        with self.assertRaises(TypeError):
            server.set_file_records(4, invalid_records)
        self.assertEqual(server.get_file_records(4), [b"one", b"two"])
        self.assertEqual(server.set_max_record_count(4, (1 << 64) - 1), 10_000)
        self.assertEqual(server.set_max_record_count(4, 1), 1)
        self.assertEqual(server.get_file_records(4), [b"one", b"two"])
        with self.assertRaises(BacnetProtocolError):
            server.set_file_data(4, b"wrong shape")
        self.assertEqual(server.get_file_records(4), [b"one", b"two"])

    def test_lookup_type_inputs_and_drained_lifecycle_fail_stably(self) -> None:
        server = make_server()
        server.add_analog_input(5, "Not a File")
        with self.assertRaisesRegex(
            ValueError, "^no pending File object with instance 5$"
        ):
            server.get_file_data(5)
        with self.assertRaisesRegex(
            ValueError, "^no pending File object with instance 99$"
        ):
            server.get_file_data(99)

        server.add_file(8, "First duplicate")
        server.set_file_data(8, b"first")
        server.add_file(8, "Effective duplicate")
        self.assertEqual(server.get_file_data(8), b"")
        server.set_file_data(8, b"effective")
        self.assertEqual(server.get_file_data(8), b"effective")

        server.add_file(6, "Strict bytes")
        invalid_data: Any = bytearray(b"not bytes")
        with self.assertRaises(TypeError):
            server.set_file_data(6, invalid_data)
        self.assertEqual(server.get_file_data(6), b"")

        drained = make_server(transport="invalid-for-artifact-test")
        drained.add_file(7, "Drained")

        async def start_and_fail() -> None:
            await drained.start()

        with self.assertRaisesRegex(RuntimeError, "^unknown transport"):
            asyncio.run(start_and_fail())
        with self.assertRaisesRegex(
            ValueError, "^no pending File object with instance 7$"
        ):
            drained.get_file_data(7)


if __name__ == "__main__":
    unittest.main()
