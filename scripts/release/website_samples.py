#!/usr/bin/env python3
"""The website's samples between and at releases (#2648).

    website_samples.py [SAMPLE ...]                          # each sample's state
    website_samples.py --pin 0.16.0 [SAMPLE ...] [--build]  # put samples on the release
    website_samples.py --follow-dev [SAMPLE ...]            # put samples on the dev branch
    website_samples.py --build [SAMPLE ...] [--work-dir D]  # build Rust samples as their manifests say
    website_samples.py --build-python 0.16.0 [SAMPLE ...]   # run Python samples on the PyPI release

The samples are website/examples/rust/* and website/examples/python/*.py,
listed in website/src/data/samples.json. A sample whose files are the
release's stays on the release; one changed since follows the `dev` branch,
and the website tests check which is which against the release's tag. A Rust
sample's manifest names each BACnet crate as `"=<version>"` (or
`{ version = "=<version>", ... }` beside other keys) on the release, or as
`{ git = "<repository>", branch = "dev" }` on dev; a Python sample on dev
has `"follows": "dev"` in samples.json. `--pin` and `--follow-dev` set the
state of the named samples, or of all. `--pin` pins each crate to its own
version at that release (bacnet-time versions on its own), read from the
workspace manifests at the tag v<version>, or from the working tree when the
tag doesn't exist yet and the workspace is at that version (the release PR).

`--build` copies each Rust sample's directory to a new directory outside the
repository and builds it there with a target directory of the run's own,
removed afterwards. `--build-python` makes a virtual environment there,
installs the release's wheel from PyPI and runs each Python sample. Each
builds or runs every sample, lists every one that failed and exits 1 if any
did. The release checklist is in docs/ci.md ("Release").
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EXAMPLES = ROOT / "website/examples"
REGISTRY = ROOT / "website/src/data/samples.json"
# website/src/lib/site.mjs's `repository`; the tests check the two agree.
REPOSITORY = "https://github.com/jscott3201/rusty-bacnet"
DEV = f'git = "{REPOSITORY}", branch = "dev"'

LINE = re.compile(r"^(bacnet-[a-z0-9-]+) = (.+)$")
VERSION = re.compile(r"\d+\.\d+\.\d+")
PINNED = re.compile(r'^"=(\d+\.\d+\.\d+)"$')
PINNED_TABLE = re.compile(r'^\{ version = "=(\d+\.\d+\.\d+)"(?:, ([^{}]*))? \}$')
DEV_TABLE = re.compile(r"^\{ " + re.escape(DEV) + r"(?:, ([^{}]*))? \}$")
# Keys that would move a crate to another source; the rest (features) stay.
SOURCE_KEY = re.compile(r"(?:^|, )(?:version|git|branch|tag|rev|path|registry|package) =")

# How --build-python runs each Python sample: by itself, with stdin at end of
# input (server_device.py stops there), or through its unittest, which
# starts the device it reads.
PYTHON_RUNS = {
    "bip_router.py": ("script",),
    "first_read.py": ("test", "crates/rusty-bacnet/tests/test_website_first_read.py"),
    "loopback_lab.py": ("script",),
    "server_device.py": ("script",),
}
PYTHON_TIMEOUT = 180


class SampleError(Exception):
    """A sample, manifest or tool this script can't handle."""


def shown(path):
    """A path as the repository names it, when it is in the repository."""
    return path.relative_to(ROOT) if inside(path, ROOT) else path


def inside(path, root):
    """Whether `path` is `root` or below it."""
    return path == root or root in path.parents


# --- the registry -------------------------------------------------------


def load_registry(path=REGISTRY):
    return json.loads(path.read_text(encoding="utf-8"))


KEY_ORDER = ("path", "language", "follows", "since", "tagPage")


def save_registry(registry, path=REGISTRY):
    """Write the registry with each sample's keys in one order, so moving a
    sample back and forth leaves the file as it was."""
    ordered = {name: {key: sample[key] for key in sorted(sample, key=lambda key: (KEY_ORDER + (key,)).index(key))}
               for name, sample in registry.items()}
    path.write_text(json.dumps(ordered, indent=2) + "\n", encoding="utf-8")


def selected(registry, names):
    """The named samples (all when none are named), refusing an unknown name."""
    unknown = sorted(set(names) - set(registry))
    if unknown:
        raise SampleError(f"no sample named {', '.join(unknown)} in {shown(REGISTRY)}")
    return [name for name in registry if not names or name in names]


# --- manifests ----------------------------------------------------------


def parse(crate, value):
    """(version, or None for dev; the other keys) of one dependency's value."""
    rest = None
    if match := PINNED.match(value):
        source, rest = match[1], ""
    elif match := PINNED_TABLE.match(value):
        source, rest = match[1], match[2] or ""
    elif match := DEV_TABLE.match(value):
        source, rest = None, match[1] or ""
    if rest is None or SOURCE_KEY.search(rest):
        raise SampleError(f'{crate} = {value}: neither {{ {DEV} }} nor "=x.y.z"')
    return source, rest


def render(source, rest):
    """A dependency's value: `source` a version, or None for the dev branch."""
    if source is None:
        return f"{{ {DEV}, {rest} }}" if rest else f"{{ {DEV} }}"
    return f'{{ version = "={source}", {rest} }}' if rest else f'"={source}"'


def bacnet_keys(document, path=()):
    """Every `bacnet-*` key in a parsed manifest, with the table it is in."""
    for key, value in document.items():
        if key.startswith("bacnet-"):
            yield path, key
        if isinstance(value, dict):
            yield from bacnet_keys(value, (*path, key))


def rewrite(text, versions=None, *, keep=False, release=None):
    """`text` with every BACnet crate pinned to `versions[crate]`, or on the
    dev branch when `versions` is None, and the state it had: each crate's
    pinned version, or None for dev. `keep` leaves it as it is."""
    section, handled, out = "", {}, []
    for line in text.split("\n"):
        if line.startswith("["):
            section = line.strip()
        match = LINE.match(line)
        if match:
            crate, value = match.groups()
            if section != "[dependencies]":
                raise SampleError(f"{crate} is in {section}, not [dependencies]")
            handled[crate], rest = parse(crate, value)
            if not keep:
                if versions is not None and crate not in versions:
                    at = f" at v{release}" if release else ""
                    raise SampleError(f"{crate} is not a published workspace crate{at}")
                line = f"{crate} = {render(None if versions is None else versions[crate], rest)}"
        out.append(line)
    try:
        document = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        raise SampleError(f"not TOML: {error}") from None
    # A BACnet crate named any other way (a dotted key, its own table, another
    # dependency table) would keep its source: refuse it.
    for path, key in bacnet_keys(document):
        if path != ("dependencies",) or key not in handled:
            raise SampleError(f"{key} in [{'.'.join(path)}]: name each BACnet crate on one `bacnet-x = ...` line of [dependencies]")
    if not handled:
        raise SampleError("the manifest names no BACnet crate")
    states = {version is None for version in handled.values()}
    if len(states) > 1:
        raise SampleError("the manifest pins some BACnet crates and takes others from dev")
    state = None if states == {True} else handled
    return "\n".join(out), state


def form(text):
    """Each crate's pinned version, or None when the manifest follows dev."""
    return rewrite(text, keep=True)[1]


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True)


def tagged(version):
    """Whether this clone has the tag v<version>."""
    return git("rev-parse", "--verify", "--quiet", f"v{version}^{{commit}}").returncode == 0


def crate_versions(version, read=None):
    """Each published workspace crate's version at release `version`: from the
    tag v<version>, or from the working tree when there is no such tag and the
    workspace is at that version (a release PR, before its tag)."""
    if read is None:
        tag = f"v{version}"
        if tagged(version):
            def read(path):
                return git("show", f"{tag}:{path}").stdout
        else:
            def read(path):
                return (ROOT / path).read_text(encoding="utf-8")
    workspace = tomllib.loads(read("Cargo.toml"))["workspace"]
    if workspace["package"]["version"] != version:
        raise SampleError(f"no tag v{version}, and the workspace is at {workspace['package']['version']}")
    versions = {}
    for member in workspace["members"]:
        package = tomllib.loads(read(f"{member}/Cargo.toml"))["package"]
        if not package["name"].startswith("bacnet-") or package.get("publish") in (False, []):
            continue
        own = package["version"]
        versions[package["name"]] = workspace["package"]["version"] if own == {"workspace": True} else own
    return versions


# --- states -------------------------------------------------------------


def state(name, sample, examples=EXAMPLES):
    """A sample's state: "dev", or "release" (with a Rust sample's pins)."""
    if sample["language"] == "rust":
        pins = form((examples / sample["path"] / "Cargo.toml").read_text(encoding="utf-8"))
        return "dev" if pins is None else "release (" + ", ".join(f"{c} ={v}" for c, v in sorted(pins.items())) + ")"
    return "dev" if sample.get("follows") == "dev" else "release"


def set_state(names, version, registry, examples=EXAMPLES, registry_path=REGISTRY, versions=None, untagged=None):
    """Put the named samples on the release `version`, or on dev when it is
    None. Nothing is written unless every manifest rewrites. Pinning to a
    release not tagged yet (the release PR) also drops the named samples'
    `tagPage` and `since` keys: that release ships each of them as a file."""
    if version is not None and versions is None:
        versions = crate_versions(version)
    if version is not None and untagged is None:
        untagged = not tagged(version)
    texts = {}
    for name in names:
        sample = registry[name]
        if sample["language"] != "rust":
            continue
        path = examples / sample["path"] / "Cargo.toml"
        try:
            texts[path] = rewrite(path.read_text(encoding="utf-8"), versions, release=version)[0]
        except SampleError as error:
            raise SampleError(f"{shown(path)}: {error}") from None
    for path, text in texts.items():
        path.write_text(text, encoding="utf-8")
    changed = False
    for name in names:
        sample = registry[name]
        if version is not None and untagged:
            for key in ("tagPage", "since"):
                changed |= sample.pop(key, None) is not None
        if sample["language"] == "rust":
            continue
        if version is None and sample.get("follows") != "dev":
            sample["follows"] = "dev"
            changed = True
        elif version is not None and "follows" in sample:
            del sample["follows"]
            changed = True
    if changed:
        save_registry(registry, registry_path)


# --- builds -------------------------------------------------------------


def work_directory(work_dir):
    parent = Path(work_dir or tempfile.gettempdir()).resolve()
    if inside(parent, ROOT):
        raise SampleError(f"{parent} is inside the repository; work outside it")
    parent.mkdir(parents=True, exist_ok=True)
    return Path(tempfile.mkdtemp(prefix="website-samples-", dir=parent))


def build(names, registry, work_dir=None, examples=EXAMPLES, run=subprocess.run):
    """Build each named Rust sample in a new directory outside the repository; the names of those that failed."""
    rust = [name for name in names if registry[name]["language"] == "rust"]
    work = work_directory(work_dir)
    failed = []
    try:
        env = {**os.environ, "CARGO_TARGET_DIR": str(work / "target")}
        try:
            cargo = run(["cargo", "--version"], cwd=work, env=env)
        except OSError as error:
            raise SampleError(f"cargo: {error}") from None
        if cargo.returncode != 0:
            raise SampleError("cargo --version failed; install Rust (https://rustup.rs)")
        for name in rust:
            source = examples / registry[name]["path"]
            copy = work / name
            try:
                shutil.copytree(source, copy, ignore=shutil.ignore_patterns("target", "Cargo.lock"))
                print(f"== {name}: {state(name, registry[name], examples)}", flush=True)
            except (OSError, SampleError) as error:
                print(f"== {name}: {error}", flush=True)
                failed.append(name)
                continue
            result = run(["cargo", "build", "--manifest-path", str(copy / "Cargo.toml")], cwd=copy, env=env)
            if result.returncode != 0:
                failed.append(name)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return failed


def venv_python(venv):
    return venv / ("Scripts/python.exe" if os.name == "nt" else "bin/python")


def python_command(name, script, python):
    """The command that runs one Python sample (with stdin at end of input)."""
    how = PYTHON_RUNS.get(name)
    if how is None:
        raise SampleError(f"{name}: add how to run it to PYTHON_RUNS")
    if how[0] == "test":
        return [str(python), "-I", "-m", "unittest", "discover", "-s", str((ROOT / how[1]).parent), "-p", Path(how[1]).name]
    return [str(python), "-I", str(script)]


def build_python(names, registry, version, work_dir=None, examples=EXAMPLES, python=sys.executable, run=subprocess.run):
    """Install rusty-bacnet==`version` from PyPI in a new virtual environment
    outside the repository and run each named Python sample with it; the
    names of those that failed."""
    scripts = [name for name in names if registry[name]["language"] == "python"]
    work = work_directory(work_dir)
    failed = []
    try:
        venv = work / "venv"
        steps = [
            [str(python), "-m", "venv", str(venv)],
            [str(venv_python(venv)), "-m", "pip", "install", "--only-binary=:all:", f"rusty-bacnet=={version}"],
        ]
        for step in steps:
            if run(step, cwd=work).returncode != 0:
                return [f"rusty-bacnet=={version} ({' '.join(step[1:4])})"]
        env = {**os.environ, "BACNET_LAB_EXPECTED_VERSION": version}
        for name in scripts:
            script = work / Path(registry[name]["path"]).name
            try:
                command = python_command(name, script, venv_python(venv))
                shutil.copy2(examples / registry[name]["path"], script)
            except (OSError, SampleError) as error:
                print(f"== {name}: {error}", flush=True)
                failed.append(name)
                continue
            print(f"== {name}: {state(name, registry[name], examples)}", flush=True)
            try:
                result = run(command, cwd=work, env=env, stdin=subprocess.DEVNULL, timeout=PYTHON_TIMEOUT)
            except subprocess.TimeoutExpired:
                print(f"== {name}: no exit within {PYTHON_TIMEOUT} s", flush=True)
                failed.append(name)
                continue
            if result.returncode != 0:
                failed.append(name)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return failed


def main(argv=None, registry_path=REGISTRY):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("samples", nargs="*", metavar="SAMPLE", help="samples.json names (default: all)")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--pin", metavar="VERSION", help="put the samples on release VERSION")
    mode.add_argument("--follow-dev", action="store_true", help="put the samples on the dev branch")
    parser.add_argument("--build", action="store_true", help="build each Rust sample outside the repository")
    parser.add_argument("--build-python", metavar="VERSION", help="run each Python sample with rusty-bacnet==VERSION from PyPI")
    parser.add_argument("--work-dir", help="where to make the build directory (default: the system's temporary directory)")
    parser.add_argument("--python", default=sys.executable, help="the interpreter --build-python makes its environment with")
    args = parser.parse_args(argv)
    try:
        for version in (args.pin, args.build_python):
            if version is not None and not VERSION.fullmatch(version):
                raise SampleError(f"a release is x.y.z, such as 0.16.0, not {version}")
        registry = load_registry(registry_path)
        names = selected(registry, args.samples)
        if args.pin is not None or args.follow_dev:
            set_state(names, args.pin, registry, registry_path=registry_path)
        for name in names:
            print(f"{name}: {state(name, registry[name])}")
        failed = []
        if args.build:
            failed += build(names, registry, args.work_dir)
        if args.build_python is not None:
            failed += build_python(names, registry, args.build_python, args.work_dir, python=args.python)
        if failed:
            print(f"::error::samples that failed: {', '.join(failed)}", file=sys.stderr)
            return 1
        if args.build or args.build_python is not None:
            print("Every sample passed.")
    except SampleError as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
