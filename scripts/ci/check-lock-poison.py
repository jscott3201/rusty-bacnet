#!/usr/bin/env python3
"""Check that non-test code takes std locks whether poisoned or not.

The rule (#2559): a std `Mutex`, `RwLock` or `Condvar` is taken whether a
panicking holder poisoned it or not, through the extension traits in
`bacnet_types::sync` (`MutexExt::locked()`, `RwLockExt::read_locked()` and
`write_locked()`, `CondvarExt::wait_locked()` and so on), so one panicking
task can't turn every later user of the lock into a second panic. Code
whose data a half-done update leaves unusable returns an error on poison
instead (`.lock().map_err(..)`), which this script leaves alone. It fails
on the spellings that panic or that write the helpers out by hand:

- `.lock()`, `.read()` or `.write()` followed by `.unwrap()` or `.expect(`.
  tokio's `lock`, `read` and `write` are async, so `.lock().unwrap()` can't
  type-check on a tokio lock, `.lock().await` never matches, and the
  pattern needs no type information. (tokio's `try_lock()` can't be
  poisoned, and `try_lock` isn't matched.)
- a Condvar wait (`.wait(..)`, `.wait_timeout(..)`, `.wait_while(..)`,
  `.wait_timeout_while(..)`) followed by `.unwrap()` or `.expect(`.
- `PoisonError::into_inner`, `|e| e.into_inner()` closures and
  `Err(e) => .. e.into_inner()` arms: the helpers written out, which
  belong in `bacnet_types::sync` only.

It reads every tracked `crates/*/src/**/*.rs` file with comments, string
literals and test-only code blanked out: items under `#[cfg(test)]`,
`#[cfg(doctest)]`, `#[cfg(feature = "test-support")]` and predicates that
imply one of them (`all(test, ..)`, `any(test, feature = "test-support")`),
`#[test]` functions, and files reached only through such a `mod x;`. Test
code, `tests/`, `benches/`, `examples/` and `fuzz/` may keep `unwrap()`.
A line can opt out with a `// lock-poison: allow` comment, which should say
why the panic is wanted.

    python3 scripts/ci/check-lock-poison.py           # check the tree
    python3 scripts/ci/check-lock-poison.py FILE...   # check these files
    python3 scripts/ci/check-lock-poison.py --self-test

Exit status: 0 when clean, 1 on a finding.
"""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ALLOW = "lock-poison: allow"
# The helpers' own definitions are the one place that spells out into_inner.
HELPER = "crates/bacnet-types/src/sync.rs"
SOURCE = re.compile(r"^crates/[^/]+/src/.+\.rs$")


TOKEN = re.compile(r"//|/\*|(?<!\w)b?r(#*)\"|\"|'")
BLOCK = re.compile(r"/\*|\*/")
STRING = re.compile(r'"(?:[^"\\]|\\.)*"', re.S)
CHAR = re.compile(r"'(?:\\(?:u\{[0-9a-fA-F_]*\}|x[0-9a-fA-F]{2}|.)|[^\\'\n])'")
NOT_NEWLINE = re.compile(r"[^\n]")


def blank_comments_and_strings(text):
    """`text` with comments and string and char literals replaced by spaces,
    newlines kept, so offsets and line numbers still line up."""
    out, i, n = [], 0, len(text)
    while m := TOKEN.search(text, i):
        start, token = m.start(), m.group(0)
        out.append(text[i:start])
        if token == "//":
            end = text.find("\n", start)
            end = n if end < 0 else end
        elif token == "/*":
            depth, end = 1, start + 2
            while depth and (k := BLOCK.search(text, end)):
                depth += 1 if k.group(0) == "/*" else -1
                end = k.end()
            end = end if not depth else n
        elif token == '"':
            literal = STRING.match(text, start)
            end = literal.end() if literal else n
        elif token == "'":
            literal = CHAR.match(text, start)
            if not literal:  # a lifetime or a loop label
                out.append("'")
                i = start + 1
                continue
            end = literal.end()
        else:  # a raw string: r"..", r#".."#, br".."
            close = '"' + m.group(1)
            end = text.find(close, m.end())
            end = n if end < 0 else end + len(close)
        out.append(NOT_NEWLINE.sub(" ", text[start:end]))
        i = end
    out.append(text[i:])
    return "".join(out)


def _split_args(s):
    parts, depth, cur = [], 0, ""
    for ch in s:
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append(cur.strip())
            cur = ""
        else:
            cur += ch
    if cur.strip():
        parts.append(cur.strip())
    return parts


def test_only(pred):
    """Whether cfg predicate `pred` can hold only in a test build."""
    pred = re.sub(r"\s+", " ", pred.strip())
    if pred in ("test", "doctest") or re.fullmatch(r'feature ?= ?"test-support"', pred):
        return True
    m = re.fullmatch(r"(all|any|not) ?\((.*)\)", pred)
    if not m:
        return False
    args = _split_args(m.group(2))
    if m.group(1) == "all":
        return any(test_only(a) for a in args)
    if m.group(1) == "any":
        return bool(args) and all(test_only(a) for a in args)
    return False


ATTR = re.compile(r"#\s*\[")
SPACE = re.compile(r"\s*")
MOD_DECL = re.compile(r"\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;")
ANY_MOD_DECL = re.compile(r"^[ \t]*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;", re.M)
TEST_ATTR = re.compile(r"(?:[\w:]+::)?test\b(?:\s*\(.*\))?", re.S)


def _attr_end(text, start):
    """Offset just past the `]` closing the attribute opened at `start`."""
    depth, j = 0, start
    while j < len(text):
        if text[j] == "[":
            depth += 1
        elif text[j] == "]":
            depth -= 1
            if depth == 0:
                return j + 1
        j += 1
    return len(text)


ITEM = re.compile(
    r"\s*(?:pub(?:\s*\([^)]*\))?\s+)?(?:(?:async|const|unsafe|extern(?:\s+\w+)?)\s+)*"
    r"(?:fn|impl|mod|struct|enum|union|trait|const|static|use|type|macro_rules!)\b"
)


def _item_end(text, start):
    """Offset just past the item, field, arm or statement at `start`: the
    first `;` (or, outside an item, `,`) at depth 0, the `}` closing its
    first top-level brace block, or the bracket that closes the enclosing one."""
    commas_end = not ITEM.match(text, start)
    depth, j = 0, start
    while j < len(text):
        ch = text[j]
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            if depth == 0:
                return j
            depth -= 1
            if ch == "}" and depth == 0:
                return j + 1
        elif depth == 0 and (ch == ";" or (ch == "," and commas_end)):
            return j + 1
        j += 1
    return len(text)


def strip_test_items(code, raw):
    """`code` (`raw` blanked by `blank_comments_and_strings`) with test-only
    items blanked too, and the names of the `mod x;` declared test-only."""
    out = list(code)
    test_mods = []
    i = 0
    while m := ATTR.search(code, i):
        start = m.start()
        end = _attr_end(code, start)
        # Read the attribute from `raw`: its strings (feature names) are blank in `code`.
        body = raw[m.end() : end - 1].strip()
        cfg = re.fullmatch(r"cfg\s*\((.*)\)", body, re.S)
        is_test = (cfg and test_only(cfg.group(1))) or TEST_ATTR.fullmatch(body)
        if not is_test:
            i = end
            continue
        # Skip any further attributes on the same item.
        j = end
        while n := ATTR.match(code, SPACE.match(code, j).end()):
            j = _attr_end(code, n.start())
        item_end = _item_end(code, j)
        decl = MOD_DECL.match(code, j, item_end)
        if decl:
            test_mods.append(decl.group(1))
        for k in range(start, item_end):
            if out[k] != "\n":
                out[k] = " "
        i = item_end
    return "".join(out), test_mods


def path_attrs(raw):
    """{module name: #[path] value} for the `mod x;` declarations of `raw`."""
    decl = r'#\[path\s*=\s*"([^"]*)"\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;'
    return {m.group(2): m.group(1) for m in re.finditer(decl, raw)}


def module_dir(path):
    """The directory that holds `path`'s child module files."""
    if path.name in ("mod.rs", "lib.rs", "main.rs"):
        return path.parent
    return path.parent / path.stem


def child_files(path, raw, names):
    """The files of `path`'s child modules `names`."""
    attrs = path_attrs(raw)
    files = []
    for name in names:
        if name in attrs:
            files.append(path.parent / attrs[name])
        else:
            files += [module_dir(path) / f"{name}.rs", module_dir(path) / name / "mod.rs"]
    return files


_LEXED = {}


def lexed(path):
    """(raw text, text with comments and literals blanked) of tracked `path`."""
    if path not in _LEXED:
        raw = (ROOT / path).read_text()
        _LEXED[path] = (raw, blank_comments_and_strings(raw))
    return _LEXED[path]


def test_files(paths):
    """The subset of `paths` reached only through a test-only `mod x;`: the
    file it names and, recursively, every module file below that one."""
    tracked = set(paths)
    queue = []
    for p in paths:
        raw, blanked = lexed(p)
        _, mods = strip_test_items(blanked, raw)
        queue += child_files(p, raw, mods)
    tests = set()
    while queue:
        c = queue.pop()
        if c in tests or c not in tracked:
            continue
        tests.add(c)
        raw, blanked = lexed(c)
        queue += child_files(c, raw, ANY_MOD_DECL.findall(blanked))
    return tests


HAND_WRITTEN = "poison recovery written out: use bacnet_types::sync's extension traits"
BAD = [
    (
        re.compile(r"\.(lock|read|write)\(\s*\)\s*\.\s*(unwrap\(\s*\)|expect\s*\()"),
        "std lock taken with {0}, which panics when it is poisoned: use {1}",
    ),
    (re.compile(r"PoisonError\s*::\s*into_inner\b"), HAND_WRITTEN),
    (re.compile(r"\|\s*(\w+)\s*\|\s*\1\s*\.\s*into_inner\(\s*\)\s*\)"), HAND_WRITTEN),
    (re.compile(r"\bErr\s*\(\s*(\w+)\s*\)\s*=>[^,;{}]*?\b\1\s*\.\s*into_inner\(\s*\)"), HAND_WRITTEN),
]
WAIT = re.compile(r"\.(wait|wait_timeout|wait_while|wait_timeout_while)\s*\(")
HELPERS = {
    "lock": "MutexExt::locked()",
    "read": "RwLockExt::read_locked()",
    "write": "RwLockExt::write_locked()",
}


def findings(path, raw, blanked=None):
    """[(line, message)] for the bad spellings in non-test code of `raw`."""
    if blanked is None:
        blanked = blank_comments_and_strings(raw)
    code, _ = strip_test_items(blanked, raw)
    lines = raw.splitlines()
    found = []

    def report(offset, message):
        line = code.count("\n", 0, offset) + 1
        if ALLOW not in lines[line - 1]:
            found.append((line, message))

    for pattern, message in BAD:
        if str(path) == HELPER and "into_inner" in pattern.pattern:
            continue
        for m in pattern.finditer(code):
            if m.lastindex and m.lastindex >= 2:
                how = "unwrap()" if m.group(2).startswith("unwrap") else "expect()"
                report(m.start(), message.format(how, HELPERS[m.group(1)]))
            else:
                report(m.start(), message)
    for m in WAIT.finditer(code):
        depth, j = 1, m.end()
        while j < len(code) and depth:
            depth += {"(": 1, ")": -1}.get(code[j], 0)
            j += 1
        if re.match(r"\s*\.\s*(unwrap\(\s*\)|expect\s*\()", code[j:]):
            report(m.start(), "Condvar wait unwrapped, which panics on poison: use CondvarExt")
    return found


def tracked_sources():
    out = subprocess.run(
        ["git", "ls-files", "crates/*.rs"], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout
    return [Path(p) for p in out.splitlines() if SOURCE.match(p)]


def check(paths):
    tests = test_files(tracked_sources())
    bad = 0
    for p in sorted(paths):
        if p in tests:
            continue
        for line, message in findings(p, *lexed(p)):
            print(f"{p}:{line}: {message}")
            bad += 1
    return bad


GOOD = """
use std::sync::{Mutex, RwLock};
use bacnet_types::sync::{MutexExt, RwLockExt};
fn f(m: &Mutex<u8>, r: &RwLock<u8>, t: &tokio::sync::Mutex<u8>) {
    let _a = m.locked();
    let _b = r.read_locked();
    let _c = r.write_locked();
    // m.lock().unwrap() in a comment is fine
    let _s = "m.lock().unwrap() in a string is fine";
}
async fn g(t: &tokio::sync::Mutex<u8>, w: &tokio::sync::RwLock<u8>) {
    let _a = t.lock().await;
    let _b = w.read().await;
    let _c = t.try_lock().unwrap();
}
fn h(m: &Mutex<u8>) {
    let _a = m.lock().unwrap(); // lock-poison: allow (the panic is wanted)
}
#[cfg(test)]
mod tests {
    fn t(m: &std::sync::Mutex<u8>) { let _a = m.lock().unwrap(); }
}
#[cfg(any(test, feature = "test-support"))]
fn support(m: &std::sync::Mutex<u8>) { let _a = m.lock().unwrap(); }
#[test]
fn one(m: &std::sync::Mutex<u8>) { let _a = m.lock().unwrap(); }
#[tokio::test(flavor = "current_thread")]
async fn two() { let m = std::sync::Mutex::new(0); let _a = m.lock().unwrap(); }
fn arms(m: &Mutex<u8>, x: u8) {
    match x {
        #[cfg(test)]
        0 => drop(m.lock().unwrap()),
        _ => {}
    }
}
"""

BAD_SAMPLES = [
    "fn f(m: &Mutex<u8>) { let _a = m.lock().unwrap(); }",
    "fn f(m: &Mutex<u8>) { let _a = m.lock().expect(\"poisoned\"); }",
    "fn f(m: &RwLock<u8>) { let _a = m\n    .read()\n    .unwrap(); }",
    "fn f(m: &RwLock<u8>) { let _a = m.write().unwrap(); }",
    "fn f(m: &Mutex<u8>) { let _a = m.lock().unwrap_or_else(PoisonError::into_inner); }",
    "fn f(m: &Mutex<u8>) { let _a = m.lock().unwrap_or_else(|e| e.into_inner()); }",
    "fn f(m: &Mutex<u8>) {\n    let _a = match m.lock() {\n        Ok(a) => a,\n        Err(p) => p.into_inner(),\n    };\n}",
    "fn f(m: &Mutex<u8>) -> u8 {\n    match m.lock() {\n        Ok(a) => *a,\n        Err(poison) => *poison.into_inner(),\n    }\n}",
    "fn f(c: &Condvar, g: G) { let _g = c.wait_timeout(g, d).unwrap(); }",
    "fn f(c: &Condvar, g: G) { let _g = c.wait_while(g, |s| s.busy()).expect(\"x\"); }",
    "#[cfg(not(test))]\nfn f(m: &Mutex<u8>) { let _a = m.lock().unwrap(); }",
    "struct S {\n    #[cfg(test)]\n    a: u8,\n    b: u8,\n}\nfn f(m: &Mutex<u8>) { m.lock().unwrap(); }",
    "fn f(m: &Mutex<u8>, x: u8) {\n    match x {\n        #[cfg(test)]\n        0 => {}\n        _ => drop(m.lock().unwrap()),\n    }\n}",
    "#[cfg(test)]\nuse std::sync::Mutex;\nfn f(m: &Mutex<u8>) { m.lock().unwrap(); }",
    "#[cfg(any(test, feature = \"std\"))]\nfn f(m: &Mutex<u8>) { let _a = m.lock().unwrap(); }",
]


def self_test():
    failures = 0
    good = findings(Path("good.rs"), GOOD)
    if good:
        print(f"self-test: the good sample was flagged: {good}")
        failures += 1
    for sample in BAD_SAMPLES:
        if not findings(Path("bad.rs"), sample):
            print(f"self-test: not flagged: {sample!r}")
            failures += 1
    print("self-test: ok" if not failures else f"self-test: {failures} failure(s)")
    return 1 if failures else 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    paths = [Path(a) for a in argv] if argv else tracked_sources()
    bad = check(paths)
    if bad:
        print(
            f"\n{bad} std lock site(s) (#2559): take the lock through bacnet_types::sync's "
            "MutexExt, RwLockExt or CondvarExt, or mark a wanted panic `// lock-poison: allow`."
        )
        return 1
    print("OK: std locks in non-test code are taken poisoned or not (bacnet_types::sync).")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
