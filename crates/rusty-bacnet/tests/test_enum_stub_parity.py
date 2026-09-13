"""Built-artifact parity for constants registered from Rust's ALL_NAMED tables."""

from __future__ import annotations

import ast
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import PropertyIdentifier


# Existing drift outside the PropertyIdentifier repair in #262. Keep this exact:
# new omissions must fail, and repaired declarations must leave this baseline.
KNOWN_MISSING_STUB_CONSTANTS = {
    "ErrorCode": {
        "BVLC_FUNCTION_UNKNOWN",
        "BVLC_PROPRIETARY_FUNCTION_UNKNOWN",
        "HEADER_ENCODING_ERROR",
        "HEADER_NOT_UNDERSTOOD",
        "INVALID_DATA_ENCODING",
        "INVALID_OPERATION_IN_THIS_STATE",
        "INVALID_OPERATOR_NAME",
        "INVALID_PARAMETER_DATA_TYPE",
        "INVALID_TIME_STAMP",
        "KEY_GENERATION_ERROR",
        "LIST_ITEM_NOT_NUMBERED",
        "LIST_ITEM_NOT_TIMESTAMPED",
        "MESSAGE_INCOMPLETE",
        "NODE_DUPLICATE_VMAC",
        "NOT_A_BACNET_SC_HUB",
        "PAYLOAD_EXPECTED",
        "SECURITY_NOT_SUPPORTED",
        "UNEXPECTED_DATA",
    },
}


def stub_classes() -> dict[str, ast.ClassDef]:
    stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    tree = ast.parse(stub_path.read_text(encoding="utf-8"), filename=str(stub_path))
    return {node.name: node for node in tree.body if isinstance(node, ast.ClassDef)}


class EnumStubParityTests(unittest.TestCase):
    def test_event_message_property_values(self) -> None:
        self.assertEqual(PropertyIdentifier.EVENT_MESSAGE_TEXTS.to_raw(), 351)
        self.assertEqual(PropertyIdentifier.EVENT_MESSAGE_TEXTS_CONFIG.to_raw(), 352)

    def test_registered_constants_match_stub(self) -> None:
        classes = stub_classes()
        # py_bacnet_enum! supplies these two methods and registers each named
        # value as an instance of its own class. Discover from the runtime, not
        # the stub, so missing declarations (including whole classes) fail.
        enum_classes = {
            name: cls
            for name, cls in vars(rusty_bacnet).items()
            if isinstance(cls, type)
            and callable(getattr(cls, "from_raw", None))
            and callable(getattr(cls, "to_raw", None))
        }
        self.assertIn("PropertyIdentifier", enum_classes)
        self.assertLessEqual(KNOWN_MISSING_STUB_CONSTANTS.keys(), enum_classes.keys())
        for name, cls in enum_classes.items():
            with self.subTest(enum=name):
                self.assertIn(name, classes)
                registered = {
                    attr for attr, value in vars(cls).items() if isinstance(value, cls)
                }
                self.assertTrue(registered, f"{name} has no registered constants")
                declarations = {
                    node.target.id: ast.unparse(node.annotation)
                    for node in classes[name].body
                    if isinstance(node, ast.AnnAssign)
                    and isinstance(node.target, ast.Name)
                    and node.target.id.isupper()
                }
                declared = set(declarations)
                self.assertSetEqual(declared - registered, set())
                self.assertSetEqual(
                    registered - declared, KNOWN_MISSING_STUB_CONSTANTS.get(name, set())
                )
                self.assertEqual(
                    declarations, {constant: name for constant in declared}
                )


if __name__ == "__main__":
    unittest.main()
