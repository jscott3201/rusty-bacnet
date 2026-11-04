"""The sample checks that need no server (#1989): the inventory file, the
Python blocks that build configuration objects or need network peers, the
example script's arguments, the host examples ``cargo`` runs, and the
installation page's dependency table.

The Python checks need ``rusty_bacnet`` installed and skip without it; the
``cargo`` check skips without ``cargo`` on PATH. Run from the repository
root: python -m unittest discover -s website/tests/python -v
"""
from __future__ import annotations

import ast
import contextlib
import importlib.util
import inspect
import io
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

import site_samples as samples
from site_samples import ApiCheck, CargoRun, Construct, ScriptArgs
from test_site_samples import TIMEOUT, block_by_key, entries, text

REPO = samples.REPO
HAS_MODULE = importlib.util.find_spec("rusty_bacnet") is not None
# What a Construct block may not hold: anything that runs, binds or waits.
NETWORK_WORDS = re.compile(r"\b(?:asyncio|await|async)\b|\.start\(")


def api_problems(source: str) -> list[str]:
    """What in a Python block doesn't match the installed ``rusty_bacnet``:
    an imported name it lacks, a call of an imported class whose arguments
    don't bind to its signature, an attribute an imported name lacks, and a
    method called on an instance of an imported class that it lacks or whose
    arguments don't bind."""
    import rusty_bacnet

    tree = ast.parse(source)
    problems, imported, instances = [], {}, {}
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom) and node.module == "rusty_bacnet":
            for alias in node.names:
                if hasattr(rusty_bacnet, alias.name):
                    imported[alias.asname or alias.name] = getattr(rusty_bacnet, alias.name)
                else:
                    problems.append(f"rusty_bacnet has no {alias.name}")

    def made(value: ast.AST) -> object | None:
        """The imported class `value` calls, if it calls one."""
        if isinstance(value, ast.Call) and isinstance(value.func, ast.Name):
            return imported.get(value.func.id)
        return None

    for node in ast.walk(tree):
        bound = []
        if isinstance(node, (ast.With, ast.AsyncWith)):
            bound = [(item.optional_vars, item.context_expr) for item in node.items]
        elif isinstance(node, ast.Assign) and len(node.targets) == 1:
            bound = [(node.targets[0], node.value)]
        for target, value in bound:
            if isinstance(target, ast.Name) and (cls := made(value)) is not None:
                instances[target.id] = cls

    def bind(callable_: object, call: ast.Call, receiver: bool, label: str) -> None:
        if any(isinstance(arg, ast.Starred) for arg in call.args) or any(kw.arg is None for kw in call.keywords):
            return
        try:
            signature = inspect.signature(callable_)
        except (TypeError, ValueError):
            problems.append(f"{label}: no signature to check")
            return
        args = [None] * (len(call.args) + receiver)
        try:
            signature.bind(*args, **{kw.arg: None for kw in call.keywords})
        except TypeError as error:
            problems.append(f"{label}: {error}")

    for node in ast.walk(tree):
        if isinstance(node, ast.Call):
            func = node.func
            if isinstance(func, ast.Name) and func.id in imported:
                bind(imported[func.id], node, False, func.id)
            elif isinstance(func, ast.Attribute) and isinstance(func.value, ast.Name) and func.value.id in instances:
                cls = instances[func.value.id]
                method = getattr(cls, func.attr, None)
                if method is None:
                    problems.append(f"{cls.__name__} has no {func.attr}")
                else:
                    bind(method, node, True, f"{cls.__name__}.{func.attr}")
        elif isinstance(node, ast.Attribute) and isinstance(node.value, ast.Name) and node.value.id in imported:
            if not hasattr(imported[node.value.id], node.attr):
                problems.append(f"{node.value.id} has no {node.attr}")
    return problems


@unittest.skipUnless(HAS_MODULE, "rusty_bacnet is not installed")
class PythonBlocks(unittest.TestCase):
    def test_construct_blocks_run_cleanly(self):
        found = entries(Construct)
        self.assertTrue(found)
        for block, _ in found:
            with self.subTest(block=block.where):
                self.assertIsNone(NETWORK_WORDS.search(block.text), "a Construct block must not run anything")
                with tempfile.TemporaryDirectory(prefix="sample-") as scratch:
                    program = Path(scratch) / "sample.py"
                    program.write_text(block.text + "\n", encoding="utf-8")
                    result = subprocess.run([sys.executable, str(program)], stdin=subprocess.DEVNULL,
                                            capture_output=True, text=True, timeout=TIMEOUT, cwd=scratch)
                output = text(result.stdout) + text(result.stderr)
                self.assertEqual((result.returncode, output), (0, ""), block.where)

    def test_api_check_blocks_match_the_module(self):
        found = entries(ApiCheck)
        self.assertTrue(found)
        for block, _ in found:
            with self.subTest(block=block.where):
                compile(block.text, block.where, "exec")
                self.assertEqual(api_problems(block.text), [])

    def test_api_check_catches_drift(self):
        good = block_by_key("connect/across-subnets.mdx::import asyncio").text
        self.assertEqual(api_problems(good), [])
        for bad, needle in (
            (good.replace("ttl=600", "tll=600"), "ForeignDevice"),
            (good.replace("ForeignDevice\n", "ForeignDevise\n", 1), "ForeignDevise"),
            (good.replace("client.foreign_registration()", "client.foreign_registrations()"), "foreign_registrations"),
            (good.replace("discover(timeout_ms=3000)", "discover(wait_ms=3000)"), "BACnetClient.discover"),
            (good.replace('BACnetClient(port=0', 'BACnetClient(prot=0'), "BACnetClient"),
        ):
            with self.subTest(needle=needle):
                self.assertNotEqual(bad, good)
                problems = api_problems(bad)
                self.assertTrue(any(needle in problem for problem in problems), problems)


def script_args(spec: ScriptArgs, args: list[str]) -> object:
    """The namespace the script's own parser makes of `args`, from the script
    loaded as a module, so its main guard doesn't run it."""
    loader = importlib.util.spec_from_file_location("site_sample_script", REPO / spec.script)
    module = importlib.util.module_from_spec(loader)
    loader.loader.exec_module(module)
    saved = sys.argv
    sys.argv = [spec.script, *args]
    try:
        return getattr(module, spec.parser)()
    finally:
        sys.argv = saved


class ScriptLines(unittest.TestCase):
    def test_script_arguments_parse(self):
        found = entries(ScriptArgs)
        self.assertTrue(found)
        for block, spec in found:
            for words in samples.commands(block.text):
                with self.subTest(block=block.where):
                    self.assertEqual(words[:2], ["python", spec.script])
                    namespace = script_args(spec, words[2:])
                    self.assertTrue(vars(namespace))
                    # The parser refuses what it doesn't define, so the check
                    # would see a renamed option.
                    with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
                        script_args(spec, [*words[2:], "--no-such-option"])


@unittest.skipUnless(shutil.which("cargo"), "cargo is not on PATH")
class CargoBlocks(unittest.TestCase):
    def test_host_examples_run(self):
        found = entries(CargoRun)
        self.assertTrue(found)
        for block, spec in found:
            lines = samples.commands(block.text)
            self.assertEqual(len(lines), len(spec.expect), f"{block.where}: one pattern set per command")
            for words, patterns in zip(lines, spec.expect, strict=True):
                with self.subTest(block=block.where, command=" ".join(words)):
                    self.assertEqual(words[:2], ["cargo", "run"])
                    self.assertIn("--example", words)
                    result = subprocess.run([*words, "--locked"], stdin=subprocess.DEVNULL, capture_output=True,
                                            text=True, timeout=600, cwd=REPO)
                    self.assertEqual(result.returncode, 0, text(result.stderr)[-2000:])
                    for pattern in patterns:
                        self.assertRegex(text(result.stdout), re.compile(pattern, re.M))


class Inventory(unittest.TestCase):
    def test_samples_md_is_current(self):
        committed = samples.INVENTORY.read_text(encoding="utf-8").replace("\r\n", "\n")
        self.assertEqual(committed, samples.inventory(),
                         "website/tests/SAMPLES.md is out of date: run python website/tests/python/site_samples.py")


class InstallationTable(unittest.TestCase):
    def test_the_installation_table_is_the_rust_quickstarts(self):
        """The installation page's Rust table, a `<Code>` component with the
        release substituted, is the Rust quickstart's, which
        first_read_source.rs checks."""
        page = (samples.DOCS / "start/installation.mdx").read_text(encoding="utf-8")
        found = re.search(r'<Code lang="toml" code=\{`(\[dependencies\].*?)`\} />', page, re.S)
        self.assertIsNotNone(found, "start/installation.mdx has no [dependencies] component")
        site = (samples.WEBSITE / "src/lib/site.mjs").read_text(encoding="utf-8")
        release = re.search(r"export const release = '([^']+)';", site).group(1)
        table = found.group(1).replace("${release}", release)
        self.assertEqual(table, block_by_key("start/rust.md::[dependencies]").text)


if __name__ == "__main__":
    unittest.main()
