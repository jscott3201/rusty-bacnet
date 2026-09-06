"""Artifact tests for the breaking Alert Enrollment source API."""

from __future__ import annotations

import ast
import inspect
import unittest
from pathlib import Path
from typing import Any

import rusty_bacnet
from rusty_bacnet import BACnetServer, ObjectIdentifier, ObjectType


def stub_method() -> ast.FunctionDef:
    stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    tree = ast.parse(stub_path.read_text(encoding="utf-8"), filename=str(stub_path))
    server = next(
        node
        for node in tree.body
        if isinstance(node, ast.ClassDef) and node.name == "BACnetServer"
    )
    method = next(
        node
        for node in server.body
        if isinstance(node, ast.FunctionDef)
        and node.name == "add_alert_enrollment"
    )
    return method


class AlertEnrollmentApiTests(unittest.TestCase):
    def test_runtime_and_stub_require_an_object_identifier_source(self) -> None:
        parameters = list(
            inspect.signature(BACnetServer.add_alert_enrollment).parameters.values()
        )
        self.assertEqual(
            [parameter.name for parameter in parameters],
            ["self", "instance", "name", "initial_source"],
        )
        self.assertTrue(
            all(parameter.default is inspect.Parameter.empty for parameter in parameters)
        )

        method = stub_method()
        stub_args = [*method.args.posonlyargs, *method.args.args]
        self.assertEqual(
            [arg.arg for arg in stub_args],
            ["self", "instance", "name", "initial_source"],
        )
        annotation = stub_args[-1].annotation
        self.assertIsNotNone(annotation)
        assert annotation is not None
        self.assertEqual(ast.unparse(annotation), "ObjectIdentifier")

    def test_object_identifier_is_converted_and_missing_or_wrong_sources_fail(self) -> None:
        server = BACnetServer(
            device_instance=420_808,
            device_name="Alert Enrollment Artifact Test",
            port=0,
        )
        source = ObjectIdentifier(ObjectType.ANALOG_INPUT, 7)
        server.add_alert_enrollment(1, "AE-1", source)

        invalid_call: Any = server.add_alert_enrollment
        with self.assertRaises(TypeError):
            invalid_call(2, "missing source")
        with self.assertRaises(TypeError):
            invalid_call(3, "wrong source", 7)


if __name__ == "__main__":
    unittest.main()
