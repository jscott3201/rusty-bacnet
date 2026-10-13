"""Seeding a log's Total_Record_Count from Python (#1537).

`add_trend_log`, `add_trend_log_multiple` and `add_event_log` take a
keyword-only ``total_record_count``. The log's first record is numbered one
past it, so a log seeded just short of 2**32 - 1 numbers its records across
the wrap to 1, and a reader's handling of the wrap can be tested without
first logging four billion records.
"""

from __future__ import annotations

import ast
import asyncio
import inspect
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)


MAX = 2**32 - 1
WAIT = 5.0
TREND_LOG = ObjectIdentifier(ObjectType.TREND_LOG, 1)
EVENT_LOG = ObjectIdentifier(ObjectType.EVENT_LOG, 1)
MULTIPLE = ObjectIdentifier(ObjectType.TREND_LOG_MULTIPLE, 1)


def installed_stub_method(name: str) -> ast.FunctionDef:
    stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    tree = ast.parse(stub_path.read_text(encoding="utf-8"), filename=str(stub_path))
    server = next(
        node
        for node in tree.body
        if isinstance(node, ast.ClassDef) and node.name == "BACnetServer"
    )
    return next(
        node
        for node in server.body
        if isinstance(node, ast.FunctionDef) and node.name == name
    )


class SeedSignatureTests(unittest.TestCase):
    def test_add_trend_log_takes_a_keyword_only_seed(self) -> None:
        parameters = inspect.signature(BACnetServer.add_trend_log).parameters
        self.assertEqual(
            list(parameters),
            ["self", "instance", "name", "buffer_size", "total_record_count"],
        )
        seed = parameters["total_record_count"]
        self.assertEqual(seed.kind, inspect.Parameter.KEYWORD_ONLY)
        self.assertEqual(seed.default, 0)
        method = installed_stub_method("add_trend_log")
        self.assertEqual(
            [argument.arg for argument in method.args.args],
            ["self", "instance", "name", "buffer_size"],
        )
        self.assertEqual(
            [argument.arg for argument in method.args.kwonlyargs], ["total_record_count"]
        )
        docs = ast.get_docstring(method)
        assert docs is not None
        self.assertIn("4294967295", docs)

    def test_a_seed_outside_unsigned32_is_refused(self) -> None:
        server = BACnetServer(1_537_000, interface="127.0.0.1", port=0)
        for seed in (-1, 2**32):
            with self.subTest(seed=seed):
                with self.assertRaises(OverflowError):
                    server.add_trend_log(1, "TL-1", total_record_count=seed)
                with self.assertRaises(OverflowError):
                    server.add_event_log(1, "EL-1", total_record_count=seed)
                with self.assertRaises(OverflowError):
                    server.add_trend_log_multiple(1, "TLM-1", total_record_count=seed)


class SeededLogLiveServerTests(unittest.TestCase):
    def test_seeded_logs_number_their_records_across_the_wrap(self) -> None:
        asyncio.run(self._exercise())

    async def _exercise(self) -> None:
        server = BACnetServer(
            1_537_001,
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        )
        server.add_analog_input(1, "AI-1", present_value=21.5)
        server.add_trend_log(1, "TL-1", total_record_count=MAX - 5)
        server.add_event_log(1, "EL-1", total_record_count=7)
        server.add_trend_log_multiple(
            1,
            "TLM-1",
            members=[
                (
                    ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
                    PropertyIdentifier.PRESENT_VALUE,
                )
            ],
            logging_type="triggered",
            total_record_count=MAX - 1,
        )
        await server.start()
        try:

            async def total(oid: ObjectIdentifier) -> PropertyValue:
                return await server.read_property(
                    oid, PropertyIdentifier.TOTAL_RECORD_COUNT
                )

            self.assertEqual(await total(TREND_LOG), PropertyValue.unsigned(MAX - 5))
            self.assertEqual(await total(EVENT_LOG), PropertyValue.unsigned(7))
            self.assertEqual(await total(MULTIPLE), PropertyValue.unsigned(MAX - 1))

            # Three triggered records: numbered 2**32 - 1, then 1 and 2.
            for expected in (MAX, 1, 2):
                await server.write_property_local(
                    MULTIPLE,
                    PropertyIdentifier.TRIGGER,
                    PropertyValue.boolean(True),
                    source_object=None,
                )
                async with asyncio.timeout(WAIT):
                    while await total(MULTIPLE) != PropertyValue.unsigned(expected):
                        await asyncio.sleep(0.02)

            address = await server.local_address()
            async with BACnetClient(
                interface="127.0.0.1", port=0, apdu_timeout_ms=2000
            ) as client:

                async def by_sequence(reference_seq: int, count: int) -> dict:
                    return await client.read_range(
                        address,
                        MULTIPLE,
                        PropertyIdentifier.LOG_BUFFER,
                        range_type="sequence",
                        reference_seq=reference_seq,
                        count=count,
                    )

                across = await by_sequence(MAX, 3)
                self.assertEqual(across["item_count"], 3)
                self.assertEqual(across["first_sequence_number"], MAX)
                self.assertEqual(across["result_flags"], (True, True, False))
                back = await by_sequence(2, -2)
                self.assertEqual(back["item_count"], 2)
                self.assertEqual(back["first_sequence_number"], 1)
                onward = await by_sequence(1, 5)
                self.assertEqual(onward["item_count"], 2)
                self.assertEqual(onward["first_sequence_number"], 1)
        finally:
            await server.stop()


if __name__ == "__main__":
    unittest.main()
