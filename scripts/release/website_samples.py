#!/usr/bin/env python3
"""The website's Rust samples at a release (#2648).

    website_samples.py                        # each manifest's form
    website_samples.py --pin 0.16.0 [--build] # pin the BACnet crates to the release
    website_samples.py --follow-dev           # take them from the dev branch again
    website_samples.py --build [--work-dir D] # build each sample as its manifest says

Between releases each sample under website/examples/rust follows the `dev`
branch: its manifest names every BACnet crate as
`{ git = "<repository>", branch = "dev" }`, and the workspace compiles its
source (bacnet-integration-tests' guide-* bins), so the page shows what
builds. The release PR runs `--pin <version>`, which rewrites each to
`"=<version>"` (or `{ version = "=<version>", ... }` beside other keys). Once
the crates are on crates.io, `--build` copies each sample to a new directory
outside the repository and builds it there, against the published crates,
with a target directory of its own, removed afterwards. It builds every
sample, lists each that failed and exits 1 if any did. After the release,
`--follow-dev` restores the dev form. The pages' notes follow the manifests
(website/src/lib/samples.mjs). The release checklist is in docs/ci.md
("Release").

Only `bacnet-*` lines in `[dependencies]` change; anything else in that
form, or a BACnet crate elsewhere, fails without writing.
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SAMPLES = ROOT / "website/examples/rust"
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


class SampleError(Exception):
    """A manifest this script can't classify or rewrite."""


def parse(crate, value):
    """(release or None for dev, the other keys) of one dependency's value."""
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
    """A dependency's value: `source` a release, or None for the dev branch."""
    if source is None:
        return f"{{ {DEV}, {rest} }}" if rest else f"{{ {DEV} }}"
    return f'{{ version = "={source}", {rest} }}' if rest else f'"={source}"'


def rewrite(text, source=None, *, keep=False):
    """`text` with every BACnet crate at `source` (a release, or None for dev),
    and the form it had: a release, or None. `keep` leaves it as it is."""
    section, found, out = "", set(), []
    for line in text.split("\n"):
        if line.startswith("["):
            section = line.strip()
        match = LINE.match(line)
        if match:
            crate, value = match.groups()
            if section != "[dependencies]":
                raise SampleError(f"{crate} is in {section}, not [dependencies]")
            current, rest = parse(crate, value)
            found.add(current)
            if not keep:
                line = f"{crate} = {render(source, rest)}"
        out.append(line)
    if not found:
        raise SampleError("the manifest names no BACnet crate")
    if len(found) > 1:
        raise SampleError("the manifest mixes " + " and ".join(sorted(v or "dev" for v in found)))
    return "\n".join(out), found.pop()


def form(text):
    """The release a manifest pins, or None when it follows dev."""
    return rewrite(text, keep=True)[1]


def manifests(samples=SAMPLES):
    """Every sample's Cargo.toml, in name order."""
    found = sorted(samples.glob("*/Cargo.toml"))
    if not found:
        raise SampleError(f"no samples under {samples}")
    return found


def set_source(source, samples=SAMPLES):
    """Rewrite every manifest; nothing is written unless all of them parse."""
    paths = manifests(samples)
    texts = {}
    for path in paths:
        try:
            texts[path] = rewrite(path.read_text(encoding="utf-8"), source)[0]
        except SampleError as error:
            raise SampleError(f"{shown(path)}: {error}") from None
    for path, text in texts.items():
        path.write_text(text, encoding="utf-8")


def inside(path, root):
    """Whether `path` is `root` or below it."""
    return path == root or root in path.parents


def shown(path):
    """A path as the repository names it, when it is in the repository."""
    return path.relative_to(ROOT) if inside(path, ROOT) else path


def build(work_dir=None, samples=SAMPLES, run=subprocess.run):
    """Build each sample in a new directory outside the repository; the names of those that failed."""
    parent = Path(work_dir or tempfile.gettempdir()).resolve()
    if inside(parent, ROOT):
        raise SampleError(f"{parent} is inside the repository; build outside it")
    parent.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="website-samples-", dir=parent))
    failed = []
    try:
        env = {**os.environ, "CARGO_TARGET_DIR": str(work / "target")}
        run(["cargo", "--version"], cwd=work, env=env)
        for manifest in manifests(samples):
            name = manifest.parent.name
            copy = work / name
            copy.mkdir()
            shutil.copy2(manifest, copy / "Cargo.toml")
            shutil.copytree(manifest.parent / "src", copy / "src")
            state = form(manifest.read_text(encoding="utf-8"))
            print(f"== {name}: BACnet crates {'=' + state if state else 'from the dev branch'}", flush=True)
            result = run(["cargo", "build", "--manifest-path", str(copy / "Cargo.toml")], cwd=copy, env=env)
            if result.returncode != 0:
                failed.append(name)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return failed


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--pin", metavar="VERSION", help="pin every BACnet crate to =VERSION")
    mode.add_argument("--follow-dev", action="store_true", help="take every BACnet crate from the dev branch")
    parser.add_argument("--build", action="store_true", help="build each sample outside the repository")
    parser.add_argument("--work-dir", help="where to make the build directory (default: the system's temporary directory)")
    args = parser.parse_args(argv)
    try:
        if args.pin is not None:
            if not VERSION.fullmatch(args.pin):
                raise SampleError(f"--pin takes a release such as 0.16.0, not {args.pin}")
            set_source(args.pin)
        elif args.follow_dev:
            set_source(None)
        states = set()
        for path in manifests():
            state = form(path.read_text(encoding="utf-8"))
            states.add(state)
            print(f"{shown(path)}: {'=' + state if state else 'dev branch'}")
        if len(states) > 1:
            raise SampleError("the manifests disagree; --pin or --follow-dev sets them all")
        if args.build:
            failed = build(args.work_dir)
            if failed:
                print(f"::error::samples that did not build: {', '.join(failed)}", file=sys.stderr)
                return 1
            print("Every sample built.")
    except SampleError as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
