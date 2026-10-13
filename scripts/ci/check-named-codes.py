#!/usr/bin/env python3
"""Check that the copies of the `named_codes!` macro stay byte-identical.

The macro builds the open wire-code newtypes of the no_std crates:
`BvlcFunction` and friends in bacnet-bip-codec, `FrameType` in
bacnet-mstp-codec, and `MessageType` and `RejectReason` in
bacnet-router-core. None of them depends on another for it, and the macro
is private to each, so each keeps a copy (#1684). This compares the
`macro_rules! named_codes { ... }` definitions, from that line to its
closing brace, against the first copy, and prints a diff for each that
differs. The doc comments above them may differ; the definitions may not.

    python3 scripts/ci/check-named-codes.py

Exit status: 0 when the copies match, 1 when one differs or can't be found.
"""

import difflib
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
COPIES = [
    "crates/bacnet-bip-codec/src/function.rs",
    "crates/bacnet-mstp-codec/src/frame.rs",
    "crates/bacnet-router-core/src/message/codes.rs",
]
START = "macro_rules! named_codes {"


def macro_definition(path):
    """The macro's definition in `path`: its first line through the brace
    that closes it, or None when the file has no definition."""
    text = (ROOT / path).read_text()
    start = text.find(START)
    if start < 0 or text.find(START, start + 1) >= 0:
        return None
    depth = 0
    for index in range(start + len(START) - 1, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return text[start : index + 1]
    return None


def main():
    definitions = {}
    for path in COPIES:
        definition = macro_definition(path)
        if definition is None:
            print(f"error: no single `{START}` definition in {path}", file=sys.stderr)
            return 1
        definitions[path] = definition
    first, *others = COPIES
    differing = [path for path in others if definitions[path] != definitions[first]]
    if not differing:
        print(f"OK: named_codes! is identical in {', '.join(COPIES)}.")
        return 0
    for path in differing:
        diff = difflib.unified_diff(
            definitions[first].splitlines(keepends=True),
            definitions[path].splitlines(keepends=True),
            fromfile=first,
            tofile=path,
        )
        sys.stdout.writelines(diff)
    print(
        "\nerror: the named_codes! copies differ; change all of them together.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
