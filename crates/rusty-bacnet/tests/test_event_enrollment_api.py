"""Artifact tests for the typed Event Enrollment and enrollment-summary APIs (#930)."""

from __future__ import annotations

import ast
import inspect
import unittest
from pathlib import Path
from typing import Any

import rusty_bacnet
from rusty_bacnet import (
    AcknowledgmentFilter,
    BACnetClient,
    BACnetServer,
    EventType,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)


def stub_method(class_name: str, method_name: str) -> ast.FunctionDef:
    stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    tree = ast.parse(stub_path.read_text(encoding="utf-8"), filename=str(stub_path))
    cls = next(
        node
        for node in tree.body
        if isinstance(node, ast.ClassDef) and node.name == class_name
    )
    return next(
        node
        for node in cls.body
        if isinstance(node, ast.FunctionDef) and node.name == method_name
    )


def stub_parameter(method: ast.FunctionDef, name: str) -> tuple[str, bool]:
    """Return a parameter's annotation and whether the stub gives it a default."""
    args = [*method.args.posonlyargs, *method.args.args]
    index = next(i for i, arg in enumerate(args) if arg.arg == name)
    annotation = args[index].annotation
    assert annotation is not None
    has_default = index >= len(args) - len(method.args.defaults)
    return ast.unparse(annotation), has_default


class EventEnrollmentApiTests(unittest.IsolatedAsyncioTestCase):
    def test_runtime_and_stub_give_event_type_a_default(self) -> None:
        parameters = inspect.signature(BACnetServer.add_event_enrollment).parameters
        self.assertEqual(
            list(parameters), ["self", "instance", "name", "event_type"]
        )
        # PyO3 renders a non-literal default as `...`; the read-back test
        # below checks which value it is.
        self.assertIsNot(parameters["event_type"].default, inspect.Parameter.empty)
        self.assertEqual(
            stub_parameter(
                stub_method("BACnetServer", "add_event_enrollment"), "event_type"
            ),
            ("EventType", True),
        )

    async def test_event_type_reads_back_including_proprietary_values(self) -> None:
        server = BACnetServer(
            device_instance=420_809,
            device_name="Event Enrollment Artifact Test",
            interface="127.0.0.1",
            port=0,
        )
        server.add_event_enrollment(1, "EE-default")
        server.add_event_enrollment(2, "EE-typed", EventType.OUT_OF_RANGE)
        server.add_event_enrollment(3, "EE-proprietary", EventType.from_raw(600))

        invalid_call: Any = server.add_event_enrollment
        with self.assertRaises(TypeError):
            invalid_call(4, "EE-int", 5)

        await server.start()
        try:
            # Event_Type follows Event_Parameters (#1868): NONE while unset.
            for instance in (1, 2, 3):
                with self.subTest(instance=instance):
                    value = await server.read_property(
                        ObjectIdentifier(ObjectType.EVENT_ENROLLMENT, instance),
                        PropertyIdentifier.EVENT_TYPE,
                    )
                    self.assertEqual(value.value, 20)
            # The legacy octet layout names no algorithm, so it serves the
            # type the enrollment was added with.
            for instance, expected in ((1, 0), (2, 5), (3, 600)):
                with self.subTest(instance=instance, legacy=True):
                    oid = ObjectIdentifier(ObjectType.EVENT_ENROLLMENT, instance)
                    await server.write_property_local(
                        oid, PropertyIdentifier.EVENT_PARAMETERS,
                        PropertyValue.octet_string(b"\x01"), source_object=None)
                    value = await server.read_property(oid, PropertyIdentifier.EVENT_TYPE)
                    self.assertEqual(value.value, expected)
        finally:
            await server.stop()


class AcknowledgmentFilterApiTests(unittest.TestCase):
    def test_constants_carry_the_request_values(self) -> None:
        self.assertEqual(
            [
                AcknowledgmentFilter.ALL.to_raw(),
                AcknowledgmentFilter.ACKED.to_raw(),
                AcknowledgmentFilter.NOT_ACKED.to_raw(),
            ],
            [0, 1, 2],
        )

    def test_runtime_and_stub_give_the_filter_a_default(self) -> None:
        parameter = inspect.signature(BACnetClient.get_enrollment_summary).parameters[
            "acknowledgment_filter"
        ]
        self.assertIsNot(parameter.default, inspect.Parameter.empty)
        self.assertEqual(
            stub_parameter(
                stub_method("BACnetClient", "get_enrollment_summary"),
                "acknowledgment_filter",
            ),
            ("AcknowledgmentFilter", True),
        )


if __name__ == "__main__":
    unittest.main()
