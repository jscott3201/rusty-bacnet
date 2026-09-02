"""Built-artifact tests for lossless AcknowledgeAlarm timestamps."""

from __future__ import annotations

import ast
import asyncio
import inspect
import unittest
import warnings
from pathlib import Path
from typing import Any

import rusty_bacnet
from rusty_bacnet import BACnetClient, BACnetTimeStamp, ObjectIdentifier, ObjectType


def stub_classes() -> dict[str, ast.ClassDef]:
    stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    tree = ast.parse(stub_path.read_text(encoding="utf-8"), filename=str(stub_path))
    return {
        node.name: node for node in tree.body if isinstance(node, ast.ClassDef)
    }


def method(class_node: ast.ClassDef, name: str) -> ast.FunctionDef | ast.AsyncFunctionDef:
    return next(
        node
        for node in class_node.body
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        and node.name == name
    )


class BACnetTimeStampArtifactTests(unittest.TestCase):
    def test_all_choices_preserve_exact_components_and_are_read_only(self) -> None:
        sequence = BACnetTimeStamp.sequence_number(65_535)
        time = BACnetTimeStamp.time(23, 59, 59, 99)
        unspecified = BACnetTimeStamp.time(255, 255, 255, 255)
        date_time = BACnetTimeStamp.date_time(
            (2154, 14, 34, 255), (0, 1, 2, 3)
        )
        unspecified_date = BACnetTimeStamp.date_time(
            (255, 255, 255, 255), (255, 255, 255, 255)
        )

        self.assertEqual((sequence.kind, sequence.value), ("sequence_number", 65_535))
        self.assertEqual((time.kind, time.value), ("time", (23, 59, 59, 99)))
        self.assertEqual(unspecified.value, (255, 255, 255, 255))
        self.assertEqual(
            date_time.value, ((2154, 14, 34, 255), (0, 1, 2, 3))
        )
        self.assertEqual(
            unspecified_date.value,
            ((255, 255, 255, 255), (255, 255, 255, 255)),
        )
        self.assertEqual(
            BACnetTimeStamp.date_time((1900, 1, 1, 1), (0, 0, 0, 0)).value,
            ((1900, 1, 1, 1), (0, 0, 0, 0)),
        )
        self.assertEqual(time, BACnetTimeStamp.time(23, 59, 59, 99))
        self.assertIn("BACnetTimeStamp.date_time", repr(date_time))
        with self.assertRaises(AttributeError):
            time.value = (1, 2, 3, 4)  # type: ignore[misc]

    def test_factories_reject_types_booleans_ranges_and_lossy_years(self) -> None:
        for invalid in (-1, 65_536, True, "1"):
            with self.subTest(sequence=invalid), self.assertRaises(ValueError):
                BACnetTimeStamp.sequence_number(invalid)  # type: ignore[arg-type]

        invalid_times = (
            (24, 0, 0, 0),
            (0, 60, 0, 0),
            (0, 0, 60, 0),
            (0, 0, 0, 100),
            (True, 0, 0, 0),
        )
        for parts in invalid_times:
            with self.subTest(time=parts), self.assertRaises(ValueError):
                BACnetTimeStamp.time(*parts)

        invalid_dates: tuple[Any, ...] = (
            [2026, 1, 1, 1],
            (2026, 1, 1),
            (1899, 1, 1, 1),
            (2155, 1, 1, 1),
            (2026, 0, 1, 1),
            (2026, 15, 1, 1),
            (2026, 1, 0, 1),
            (2026, 1, 35, 1),
            (2026, 1, 1, 0),
            (2026, 1, 1, 8),
            (True, 1, 1, 1),
        )
        for date in invalid_dates:
            with self.subTest(date=date), self.assertRaises(ValueError):
                BACnetTimeStamp.date_time(date, (0, 0, 0, 0))
        with self.assertRaisesRegex(
            ValueError, "^time must be a tuple of exactly 4 integers"
        ):
            invalid_time: Any = (0, 0, 0)
            BACnetTimeStamp.date_time((2026, 1, 1, 1), invalid_time)

    def test_runtime_stub_and_packaging_are_in_parity(self) -> None:
        self.assertIs(getattr(rusty_bacnet, "BACnetTimeStamp"), BACnetTimeStamp)
        module_path = Path(rusty_bacnet.__file__)
        stub_path = module_path.with_suffix(".pyi")
        self.assertTrue(stub_path.is_file())
        self.assertTrue(module_path.with_name("py.typed").is_file())
        classes = stub_classes()
        self.assertIn("BACnetTimeStamp", classes)

        runtime_parameters = list(
            inspect.signature(BACnetClient.acknowledge_alarm_request).parameters.values()
        )
        expected = [
            "self",
            "address",
            "acknowledging_process_identifier",
            "event_object_identifier",
            "event_state_acknowledged",
            "timestamp",
            "acknowledgment_source",
            "time_of_acknowledgment",
        ]
        self.assertEqual([parameter.name for parameter in runtime_parameters], expected)
        self.assertTrue(
            all(parameter.default is inspect.Parameter.empty for parameter in runtime_parameters)
        )

        client_method = method(classes["BACnetClient"], "acknowledge_alarm_request")
        stub_args = [*client_method.args.posonlyargs, *client_method.args.args]
        self.assertEqual([arg.arg for arg in stub_args], expected)
        timestamp_annotation = stub_args[5].annotation
        acknowledgment_annotation = stub_args[7].annotation
        self.assertIsNotNone(timestamp_annotation)
        self.assertIsNotNone(acknowledgment_annotation)
        assert timestamp_annotation is not None
        assert acknowledgment_annotation is not None
        self.assertEqual(ast.unparse(timestamp_annotation), "BACnetTimeStamp")
        self.assertEqual(ast.unparse(acknowledgment_annotation), "BACnetTimeStamp")

    def test_both_canonical_timestamps_are_mandatory(self) -> None:
        client = BACnetClient()
        oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        timestamp = BACnetTimeStamp.sequence_number(1)
        call: Any = client.acknowledge_alarm_request
        with self.assertRaises(TypeError):
            call("127.0.0.1:47808", 7, oid, 1, timestamp, "operator")
        with self.assertRaises(TypeError):
            call(
                "127.0.0.1:47808",
                7,
                oid,
                1,
                0,
                "operator",
                timestamp,
            )

    def test_legacy_signature_and_warning_precede_lifecycle_failure(self) -> None:
        expected = [
            "self",
            "address",
            "acknowledging_process_identifier",
            "event_object_identifier",
            "event_state_acknowledged",
            "acknowledgment_source",
        ]
        parameters = list(inspect.signature(BACnetClient.acknowledge_alarm).parameters.values())
        self.assertEqual([parameter.name for parameter in parameters], expected)

        async def invoke() -> None:
            client = BACnetClient()
            oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
            with warnings.catch_warnings(record=True) as caught:
                warnings.simplefilter("always")
                with self.assertRaisesRegex(RuntimeError, "client not started"):
                    await client.acknowledge_alarm(
                        "127.0.0.1:47808", 7, oid, 1, "legacy-operator"
                    )
                self.assertEqual(len(caught), 1)
                self.assertIs(caught[0].category, DeprecationWarning)
                self.assertIn("acknowledge_alarm_request", str(caught[0].message))

        asyncio.run(invoke())


if __name__ == "__main__":
    unittest.main()
