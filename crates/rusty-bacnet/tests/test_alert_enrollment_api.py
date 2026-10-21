"""Artifact tests for the breaking Alert Enrollment source API."""

from __future__ import annotations

import ast
import inspect
import unittest
from pathlib import Path
from typing import Any

import rusty_bacnet
from rusty_bacnet import (BACnetServer, ObjectIdentifier, ObjectType, PropertyIdentifier,
                          PropertyValue)


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


class AlertEnrollmentReportingOptionTests(unittest.IsolatedAsyncioTestCase):
    async def test_reporting_options_are_served_and_written(self) -> None:
        # Table 12-61 lists the message texts and the inhibit pair (#1865).
        P = PropertyIdentifier
        server = BACnetServer(9601, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1")
        server.add_alert_enrollment(1, "AE-1", ObjectIdentifier(ObjectType.ANALOG_INPUT, 7))
        await server.start()
        try:
            alert = ObjectIdentifier(ObjectType.ALERT_ENROLLMENT, 1)
            empty = PropertyValue.list([PropertyValue.character_string("")] * 3)
            self.assertEqual(await server.read_property(alert, P.EVENT_MESSAGE_TEXTS), empty)
            config = PropertyValue.list([PropertyValue.character_string(text)
                                         for text in ("Alert", "", "Clear")])
            await server.write_property_local(alert, P.EVENT_MESSAGE_TEXTS_CONFIG, config,
                                              source_object=None)
            self.assertEqual(await server.read_property(alert, P.EVENT_MESSAGE_TEXTS_CONFIG),
                             config)
            await server.write_property_local(alert, P.EVENT_ALGORITHM_INHIBIT,
                                              PropertyValue.boolean(True), source_object=None)
            inhibit = await server.read_property(alert, P.EVENT_ALGORITHM_INHIBIT)
            self.assertEqual((inhibit.tag, inhibit.value), ("boolean", True))
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
