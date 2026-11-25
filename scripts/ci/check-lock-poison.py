#!/usr/bin/env python3
"""Check that non-test code never panics on a poisoned std lock.

The rule (#2559): a std `Mutex`, `RwLock` or `Condvar` poisoned by a panic is
taken anyway, through the extension traits in `bacnet_types::sync`
(`MutexExt::locked()`, `RwLockExt::read_locked()` and `write_locked()`,
`CondvarExt::wait_locked()` and so on), so one panicking task can't make
every later user of the lock panic too. Where a half-done update must not be
used, the code fails closed instead, returning an error or doing nothing, and
says why with `// lock-poison: fail-closed (<why>)`. A wanted panic, which
nothing here has, would say so with `// lock-poison: allow (<why>)`. A marker
counts on the flagged line, earlier in the same statement, or in the comment
lines just above it. The script fails on:

- `.lock()`, `.read()` or `.write()` followed by `.unwrap()` or `.expect(`,
  also through `.ok()`, through the type (`Mutex::lock(&m).unwrap()`) or
  through a binding (`let g = m.lock();` then `g.unwrap()`), a zero-argument
  `.get_mut()` unwrapped, and an unwrapped Condvar wait. tokio's `lock`,
  `read` and `write` are async, so none of these can type-check on a tokio
  lock and the patterns need no type information; `.lock().await` never
  matches, and `try_lock` isn't looked at.
- `PoisonError::into_inner`, `|e| e.into_inner()` closures,
  `Err(e) => .. e.into_inner()` arms and `TryLockError::Poisoned`: the
  helpers written out, which belong in `bacnet_types::sync` only.
- a lock result looked at instead of taken, unless marked fail-closed with a
  reason: a Result method (`.ok()`, `.map_err(..)`, `.is_ok_and(..)` and the
  like) or `?` after the call, or the call as the scrutinee of `if let Ok`,
  `let Ok(..) else` or `match`. The marked sites are listed with their
  reasons.

It reads every tracked `.rs` file the workspace builds or ships as a sample
(crates, their examples and build scripts, the benchmark tools, examples/,
firmware/, website/examples/) except tests/, benches/, fuzz/ and the archived
benchmarks/results/, with comments, string literals and test-only code
blanked out: items under `#[cfg(test)]`, `#[cfg(doctest)]`,
`#[cfg(feature = "test-support")]` and predicates that imply one of them
(`all(test, ..)`, `any(test, feature = "test-support")`), `#[test]`
functions, and files reached only through such a `mod x;`. Test code may
unwrap.

    python3 scripts/ci/check-lock-poison.py           # check the tree
    python3 scripts/ci/check-lock-poison.py FILE...   # check these files
    python3 scripts/ci/check-lock-poison.py --self-test

Exit status: 0 when clean, 1 on a finding or when there was nothing to read.
"""

import contextlib
import io
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# The helpers' own definitions are the one place that spells out into_inner.
HELPER = "crates/bacnet-types/src/sync.rs"


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


ALLOW = re.compile(r"lock-poison: allow\b")
FAIL_CLOSED = re.compile(r"lock-poison: fail-closed\b(?: \((?P<why>[^)]*\w[^)]*)\))?")
PANIC = r"(?P<panic>unwrap\s*\(\s*\)|expect\s*\()"
LOCK = r"\.\s*(?P<method>lock|read|write)\s*\(\s*\)"
HELPERS = {
    "lock": "MutexExt::locked()",
    "read": "RwLockExt::read_locked()",
    "write": "RwLockExt::write_locked()",
}
HAND_WRITTEN = "poison recovery written out: use bacnet_types::sync's extension traits"
INTO_INNER = (
    r"unwrap_or_else\s*\(\s*(?:(?:std\s*::\s*)?sync\s*::\s*)?PoisonError\s*::\s*into_inner\s*\)"
    r"|unwrap_or_else\s*\(\s*\|\s*(?P<v>\w+)\s*\|\s*(?P=v)\s*\.\s*into_inner\(\s*\)\s*\)"
)
# A lock call whose result is unwrapped, directly or through `.ok()`.
PANICS = re.compile(LOCK + r"\s*\.\s*(?:ok\s*\(\s*\)\s*\.\s*)?" + PANIC)
# The same through a type path: `Mutex::lock(&m).unwrap()`.
UFCS = re.compile(r"\b\w*(?:Mutex|RwLock)\s*::\s*(?P<method>lock|read|write)\s*\(")
GET_MUT = re.compile(r"\.\s*get_mut\s*\(\s*\)\s*\.\s*" + PANIC)
WAIT = re.compile(r"\.\s*(?:wait|wait_timeout|wait_while|wait_timeout_while)\s*\(")
FOLLOWED_BY_PANIC = re.compile(r"\s*\.\s*" + PANIC)
# `let guard = m.lock();` and then `guard.unwrap()`.
BOUND = re.compile(r"\blet\s+(?:mut\s+)?(?P<name>\w+)\s*(?::[^=;]*)?=[^;]*?" + LOCK + r"\s*;")
REBIND = re.compile(r"\blet\s+(?:mut\s+)?$")
HAND = [
    re.compile(r"PoisonError\s*::\s*into_inner\b"),
    re.compile(r"\bTryLockError\s*::\s*Poisoned\b"),
    re.compile(r"\|\s*(\w+)\s*\|\s*\1\s*\.\s*into_inner\(\s*\)\s*\)"),
    re.compile(r"\bErr\s*\(\s*(\w+)\s*\)\s*=>[^,;{}]*?\b\1\s*\.\s*into_inner\(\s*\)"),
]
ERR_ARM_BLOCK = re.compile(r"\bErr\s*\(\s*(?P<name>\w+)\s*\)\s*=>\s*\{")
# A lock result looked at instead of taken: a Result method or `?` after the
# call, or the call as an `if let Ok(..)`, `let Ok(..) else` or `match`
# scrutinee.
RESULT_METHOD = re.compile(
    r"\s*\.\s*(?:ok|err|is_ok|is_err|is_ok_and|is_err_and|map|map_err|map_or|map_or_else"
    r"|and_then|or_else|unwrap_or|unwrap_or_default|unwrap_or_else|inspect|inspect_err)\b"
)
INTO_INNER_AFTER = re.compile(r"\s*\.\s*(?:" + INTO_INNER + ")")
QUESTION = re.compile(r"\s*\?")
SCRUTINEE_END = re.compile(r"\s*(?:\{|else\b|&&)")
LET_OK = re.compile(r"\blet\s+(?:Ok|Err)\s*\(")
# `match` on the call; `match &*m.lock()` derefs a guard, not a Result.
MATCH = re.compile(r"\bmatch\s+(?!&?\s*\*)")
STATEMENT_END = re.compile(r"[;{}]\s*$")


def _close_paren(code, j):
    """Offset just past the `)` that closes the `(` before offset `j`."""
    depth = 1
    while j < len(code) and depth:
        depth += {"(": 1, ")": -1}.get(code[j], 0)
        j += 1
    return j


def _statement_start(code, offset):
    """Offset of the statement holding `offset`: just past the last `;`,
    `{` or `}` before it."""
    k = max(code.rfind(";", 0, offset), code.rfind("{", 0, offset), code.rfind("}", 0, offset))
    return k + 1


def _marked(raw_lines, code_lines, line, marker):
    """The `marker` match on `line` (1-based), on the statement's earlier
    lines, or in the comment lines just above the statement; else None."""
    k = line - 1
    while k >= 0 and line - k <= 12:
        if m := marker.search(raw_lines[k]):
            return m
        if k < line - 1 and STATEMENT_END.search(code_lines[k]):
            return None
        k -= 1
    return None


def findings(path, raw, blanked=None):
    """([(line, message)] for the bad spellings in non-test code of `raw`,
    [(line, reason)] for the fail-closed sites marked with one)."""
    if blanked is None:
        blanked = blank_comments_and_strings(raw)
    code, _ = strip_test_items(blanked, raw)
    raw_lines, code_lines = raw.splitlines(), code.splitlines()
    helper = str(path) == HELPER
    found = {}
    closed = []

    def report(offset, message, marker=ALLOW):
        line = code.count("\n", 0, offset) + 1
        if line in found or _marked(raw_lines, code_lines, line, marker):
            return
        found[line] = message

    for m in PANICS.finditer(code):
        how = "unwrap()" if m.group("panic").startswith("unwrap") else "expect()"
        report(
            m.start(),
            f"std lock taken with {how}, which panics when it is poisoned: "
            f"use {HELPERS[m.group('method')]}",
        )
    for m in UFCS.finditer(code):
        if FOLLOWED_BY_PANIC.match(code, _close_paren(code, m.end())):
            report(
                m.start(),
                "std lock taken through its type and unwrapped: "
                f"use {HELPERS[m.group('method')]}",
            )
    for m in GET_MUT.finditer(code):
        report(
            m.start(),
            "get_mut() unwrapped, which panics when the lock is poisoned: use get_mut_locked()",
        )
    for m in WAIT.finditer(code):
        if FOLLOWED_BY_PANIC.match(code, _close_paren(code, m.end())):
            report(m.start(), "Condvar wait unwrapped, which panics on poison: use CondvarExt")
    for m in BOUND.finditer(code):
        # The binding's first use, past any `let` that names it again.
        for use in re.compile(r"\b" + m.group("name") + r"\b").finditer(code, m.end()):
            if REBIND.search(code[max(0, use.start() - 16) : use.start()]):
                continue
            if FOLLOWED_BY_PANIC.match(code, use.end()):
                report(
                    use.start(),
                    f"std lock result `{m.group('name')}` unwrapped, which panics when it "
                    f"is poisoned: use {HELPERS[m.group('method')]}",
                )
            break
    if not helper:
        for pattern in HAND:
            for m in pattern.finditer(code):
                report(m.start(), HAND_WRITTEN)
        for m in ERR_ARM_BLOCK.finditer(code):
            depth, j = 1, m.end()
            while j < len(code) and depth:
                depth += {"{": 1, "}": -1}.get(code[j], 0)
                j += 1
            body = code[m.end() : j]
            if re.search(r"\b" + m.group("name") + r"\s*\.\s*into_inner\(\s*\)", body):
                report(m.start(), HAND_WRITTEN)
    for m in re.finditer(LOCK, code):
        after = m.end()
        inspected = (
            RESULT_METHOD.match(code, after) and not INTO_INNER_AFTER.match(code, after)
        ) or QUESTION.match(code, after)
        if not inspected and SCRUTINEE_END.match(code, after):
            before = code[_statement_start(code, m.start()) : m.start()]
            inspected = LET_OK.search(before) or MATCH.search(before)
        if not inspected:
            continue
        line = code.count("\n", 0, m.start()) + 1
        mark = _marked(raw_lines, code_lines, line, FAIL_CLOSED)
        if mark and mark.group("why"):
            closed.append((line, mark.group("why")))
        elif mark:
            found.setdefault(
                line, "fail-closed marker without a reason: `// lock-poison: fail-closed (<why>)`"
            )
        else:
            report(
                m.start(),
                "std lock result inspected instead of taken: take it (MutexExt/RwLockExt), or "
                "fail closed with `// lock-poison: fail-closed (<why>)` where a half-done "
                "update must not be used",
            )
    return sorted(found.items()), closed


# Tracked Rust the workspace builds or ships as samples. Tests, benches and
# fuzz targets may unwrap; benchmarks/results/ holds archived snapshots.
EXCLUDED = re.compile(r"(^|/)(tests|benches|fuzz)/|^benchmarks/results/")


def tracked_sources():
    out = subprocess.run(
        ["git", "ls-files", "*.rs"], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout
    return [Path(p) for p in out.splitlines() if not EXCLUDED.search(p)]


def check(paths):
    """(files read, findings printed, ["file:line: reason"] of the marked
    fail-closed sites)."""
    tests = test_files(tracked_sources())
    read = bad = 0
    closed = []
    for p in sorted(paths):
        if p in tests:
            continue
        read += 1
        found, marked = findings(p, *lexed(p))
        closed += [f"{p}:{line}: {why}" for line, why in marked]
        for line, message in found:
            print(f"{p}:{line}: {message}")
            bad += 1
    return read, bad, closed


GOOD = """
use std::sync::{Mutex, RwLock};
use bacnet_types::sync::{MutexExt, RwLockExt};
fn f(m: &Mutex<u8>, r: &RwLock<u8>, t: &tokio::sync::Mutex<u8>) {
    let _a = m.locked();
    let _b = r.read_locked();
    let _c = r.write_locked();
    // m.lock().unwrap() in a comment is fine
    let _s = "m.lock().unwrap() in a string is fine";
    let _d = std::fs::read(path).unwrap();
}
async fn g(t: &tokio::sync::Mutex<u8>, w: &tokio::sync::RwLock<u8>) {
    let _a = t.lock().await;
    let _b = w.read().await;
    let _c = t.try_lock().unwrap();
    let pending = t.lock();
    let _d = pending.await;
    let _e = tokio::time::timeout(d, w.write()).await;
}
fn h(m: &Mutex<u8>) -> Result<(), Error> {
    let _a = m.lock().unwrap(); // lock-poison: allow (the panic is wanted)
    // lock-poison: fail-closed (a half-done lease could repeat an ID)
    let _b = m
        .lock()
        .map_err(|_| Error::Poisoned)?;
    // lock-poison: fail-closed (poisoned acts closed)
    if let Ok(mut g) = m.lock() {
        *g += 1;
    }
    Ok(())
}
fn persisted(file: &Durable) -> Result<(), Error> {
    let Some(octets) = file.read_octets()? else { return Ok(()) };
    let _ = match file.read_octets() { Ok(x) => x, Err(e) => return Err(e) };
    Ok(())
}
fn try_take(m: &Mutex<u8>) -> Option<MutexGuard<'_, u8>> {
    m.try_locked()
}
#[cfg(test)]
mod tests {
    fn t(m: &std::sync::Mutex<u8>) { let _a = m.lock().unwrap(); let _b = m.lock().ok(); }
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
fn guard(table: &SyncMutex<Option<u8>>) -> u8 {
    match &*table.lock() {
        Some(x) => *x,
        None => 0,
    }
}
"""

BAD_SAMPLES = [
    "fn f(m: &Mutex<u8>) { let _a = m.lock().unwrap(); }",
    'fn f(m: &Mutex<u8>) { let _a = m.lock().expect("poisoned"); }',
    "fn f(m: &RwLock<u8>) { let _a = m\n    .read()\n    .unwrap(); }",
    "fn f(m: &RwLock<u8>) { let _a = m.write().unwrap(); }",
    "fn f(m: &Mutex<u8>) { let _a = m.lock().ok().unwrap(); }",
    "fn f(m: &Mutex<u8>) { let _a = Mutex::lock(&m).unwrap(); }",
    'fn f(m: &RwLock<u8>) { let _a = std::sync::RwLock::write(&m).expect("x"); }',
    "fn f(m: &mut Mutex<u8>) { *m.get_mut().unwrap() = 1; }",
    "fn f(m: &Mutex<u8>) {\n    let g = m.lock();\n    let g = g.unwrap();\n}",
    "fn f(m: &Mutex<u8>) { let _a = m.lock().unwrap_or_else(PoisonError::into_inner); }",
    "fn f(m: &Mutex<u8>) { let _a = m.lock().unwrap_or_else(std::sync::PoisonError::into_inner); }",
    "fn f(m: &Mutex<u8>) { let _a = m.lock().unwrap_or_else(|e| e.into_inner()); }",
    "fn f(m: &Mutex<u8>) {\n    let _a = match m.lock() {\n        Ok(a) => a,\n"
    "        Err(p) => p.into_inner(),\n    };\n}",
    "fn f(m: &Mutex<u8>) -> u8 {\n    match m.lock() {\n        Ok(a) => *a,\n"
    "        Err(poison) => *poison.into_inner(),\n    }\n}",
    "fn f(m: &Mutex<u8>) {\n    let _a = match m.lock() {\n        Ok(a) => a,\n"
    "        Err(p) => {\n            warn();\n            p.into_inner()\n        }\n    };\n}",
    "fn f(c: &Condvar, g: G) { let _g = c.wait_timeout(g, d).unwrap(); }",
    'fn f(c: &Condvar, g: G) { let _g = c.wait_while(g, |s| s.busy()).expect("x"); }',
    "fn f(m: &Mutex<u8>) -> Option<u8> { Some(*m.lock().ok()?) }",
    "fn f(m: &Mutex<u8>) -> Result<(), E> { let _a = m.lock().map_err(|_| E)?; Ok(()) }",
    "fn f(m: &Mutex<u8>) -> bool { m.lock().is_ok_and(|g| *g > 0) }",
    "fn f(m: &RwLock<u8>) -> bool { m.read().map_or(false, |g| *g > 0) }",
    "fn f(m: &RwLock<u8>) -> Result<u8, E> { Ok(*m.read()?) }",
    "fn f(m: &RwLock<u8>) {\n    match m.write() {\n        Ok(g) => drop(g),\n        Err(_) => {}\n    }\n}",
    "fn f(m: &Mutex<u8>) -> Option<MutexGuard<'_, u8>> {\n    match m.try_lock() {\n"
    "        Ok(g) => Some(g),\n        Err(TryLockError::Poisoned(p)) => Some(p.into_inner()),\n"
    "        Err(TryLockError::WouldBlock) => None,\n    }\n}",
    "fn f(m: &Mutex<u8>) {\n    if let Ok(mut g) = m.lock() {\n        *g += 1;\n    }\n}",
    "fn f(m: &RwLock<u8>) {\n    let Ok(g) = m.write() else {\n        return;\n    };\n}",
    "fn f(m: &Mutex<u8>) {\n    match m.lock() {\n        Ok(g) => drop(g),\n        Err(_) => {}\n    }\n}",
    "fn f(m: &Mutex<u8>) {\n    // lock-poison: fail-closed\n    if let Ok(g) = m.lock() {\n"
    "        drop(g);\n    }\n}",
    "fn f(m: &Mutex<u8>) {\n    // lock-poison: fail-closed (why)\n    let a = 1;\n"
    "    if let Ok(g) = m.lock() {\n        drop(g);\n    }\n}",
    "#[cfg(not(test))]\nfn f(m: &Mutex<u8>) { let _a = m.lock().unwrap(); }",
    "struct S {\n    #[cfg(test)]\n    a: u8,\n    b: u8,\n}\nfn f(m: &Mutex<u8>) { m.lock().unwrap(); }",
    "fn f(m: &Mutex<u8>, x: u8) {\n    match x {\n        #[cfg(test)]\n        0 => {}\n"
    "        _ => drop(m.lock().unwrap()),\n    }\n}",
    "#[cfg(test)]\nuse std::sync::Mutex;\nfn f(m: &Mutex<u8>) { m.lock().unwrap(); }",
    '#[cfg(any(test, feature = "std"))]\nfn f(m: &Mutex<u8>) { let _a = m.lock().unwrap(); }',
]


def self_test():
    failures = 0
    good, closed = findings(Path("good.rs"), GOOD)
    if good:
        print(f"self-test: the good sample was flagged: {good}")
        failures += 1
    if [why for _, why in closed] != [
        "a half-done lease could repeat an ID",
        "poisoned acts closed",
    ]:
        print(f"self-test: the good sample's fail-closed sites read {closed}")
        failures += 1
    for sample in BAD_SAMPLES:
        if not findings(Path("bad.rs"), sample)[0]:
            print(f"self-test: not flagged: {sample!r}")
            failures += 1
    # A pathspec that finds nothing must fail, not print OK.
    global tracked_sources
    tracked = tracked_sources
    tracked_sources = list
    try:
        with contextlib.redirect_stdout(io.StringIO()):
            empty = main([])
        if empty != 1:
            print("self-test: an empty scan passed")
            failures += 1
    finally:
        tracked_sources = tracked
    print("self-test: ok" if not failures else f"self-test: {failures} failure(s)")
    return 1 if failures else 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    paths = [Path(a) for a in argv] if argv else tracked_sources()
    if not paths:
        print("error: no Rust files to check (did the pathspec change?)")
        return 1
    read, bad, closed = check(paths)
    if bad:
        print(
            f"\n{bad} std lock site(s) (#2559): take the lock through bacnet_types::sync's "
            "MutexExt, RwLockExt or CondvarExt; fail closed with `// lock-poison: fail-closed "
            "(<why>)`; or mark a wanted panic `// lock-poison: allow`."
        )
        return 1
    print(f"{len(closed)} lock site(s) fail closed on poison:")
    for site in closed:
        print(f"  {site}")
    print(
        f"OK: {read} files read; std locks are taken poisoned or not (bacnet_types::sync), "
        f"or fail closed where marked."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
