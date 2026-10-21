#!/usr/bin/env python3
"""Write THIRD-PARTY-NOTICES for the release binaries (#943, #944, #1472).

    third_party_notices.py --out THIRD-PARTY-NOTICES [--libpcap DIR]

Lists what the release binaries link statically, with where to get each
component's source and every licence file it ships:
- the Rust crates in the bacnet CLI and in the rusty_bacnet Python extension,
  for every release target (TARGETS, with the CLI's features on each):
  `cargo tree` with normal edges only, so build scripts, proc-macros and
  dev-dependencies, which end up in no binary, are left out, and so are the
  crates of targets that aren't released. A crate's source is its crates.io
  page for that version (or, for a crate from elsewhere, its repository);
- libpcap, which the Linux CLI links statically (--libpcap: a directory with
  its LICENSE and VERSION, which scripts/release/libpcap.sh writes).

Identical licence texts are printed once, followed by every crate that ships
them. Generation fails if a crate under a licence that needs its notice kept
(anything but the ones in NO_NOTICE, whose terms don't ask for it in a
binary) ships no licence file,
unless ALLOW_NO_LICENSE_FILE names it with a reason. The output depends only
on Cargo.lock, the crate sources and libpcap, so a rebuild produces the same
file. Run `cargo fetch --locked` first: cargo runs with --offline here.
"""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

# Each release target with the CLI's features there, on top of its default
# ones (the terminal UI, #974): packet capture (pcap) and MS/TP (serial, #969)
# only on Linux, as in scripts/release/build_linux.sh. The Python extension has
# the same features everywhere.
TARGETS = {
    "x86_64-unknown-linux-gnu": "sc-tls,pcap,serial",
    "aarch64-unknown-linux-gnu": "sc-tls,pcap,serial",
    "x86_64-apple-darwin": "sc-tls",
    "aarch64-apple-darwin": "sc-tls",
    "x86_64-pc-windows-msvc": "sc-tls",
}
LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|copyright|notice|unlicense)([-._].*)?$", re.IGNORECASE)
SOURCE_SUFFIXES = {".rs", ".py", ".toml", ".json", ".sh", ".c", ".h", ".yml", ".yaml", ".html"}
# Licences below a crate's root that cover code compiled into it, as (path,
# kind, what it covers): kind "file" is a licence file, "comment" the licence
# in a C source file's leading comment.
EXTRA_LICENSES = {
    "aws-lc-sys": [
        # The bundled AWS-LC C library, with its summary of the third-party code in it.
        ("aws-lc/LICENSE", "file", None),
        ("aws-lc/third_party/fiat/LICENSE", "file", "fiat-crypto, compiled into AWS-LC"),
        # The crate doesn't ship jitterentropy's LICENSE; its header carries the
        # licence, and aws-lc/LICENSE says which of its terms AWS-LC elects.
        ("aws-lc/third_party/jitterentropy/jitterentropy-library/jitterentropy.h", "comment",
         "the licence of the jitterentropy library, built into AWS-LC on Linux and Windows, whose"
         " BSD-3-Clause terms AWS-LC elects"),
    ],
}
# Licence texts kept in scripts/release/licenses/ for crates whose published
# package leaves out the licence file its repository has: {name: (version,
# file, where the text came from)}. The version is pinned, so a bump has to
# check the new release's licence again.
VENDORED_LICENSES = {
    "tui-tree-widget": ("0.24.1", "tui-tree-widget-0.24.1-LICENSE",
                        "LICENSE from github.com/EdJoPaTo/tui-rs-tree-widget at tag v0.24.1"),
}
VENDORED_DIR = Path(__file__).resolve().parent / "licenses"
# SPDX identifiers whose terms don't ask for the notice to go with a binary.
# Any other identifier, unknown ones included, counts as needing its notice.
# BSL-1.0 asks for it in copies of the software except machine-executable
# object code, so not in a binary.
NO_NOTICE = {"0BSD", "BSL-1.0", "CC0-1.0", "MIT-0", "Unlicense", "WTFPL"}
# Crates that may ship no licence file although their licence needs its notice:
# {name: why that's fine}. Empty: every such crate ships one.
ALLOW_NO_LICENSE_FILE = {}
CRATES_IO = {"registry+https://github.com/rust-lang/crates.io-index", "sparse+https://index.crates.io/"}
SPDX_WORD = re.compile(r"[A-Za-z0-9.+-]+")
TREE_LINE = re.compile(r"^(\S+) v(\S+)")
RULE = "=" * 79


class NoticesError(Exception):
    """The notices can't be generated faithfully."""


def cargo(*args):
    return subprocess.run(["cargo", *args], check=True, capture_output=True, text=True).stdout


def parse_tree(text):
    """{(name, version)} from `cargo tree --prefix none --format {p}` output."""
    return {m.groups() for m in map(TREE_LINE.match, text.splitlines()) if m}


def linked_crates():
    """{(name, version): {artifact labels}} for every crate linked into a release binary."""
    used = {}
    for target, cli_features in TARGETS.items():
        artifacts = {"CLI": ["-p", "bacnet-cli", "--features", cli_features], "Python": ["-p", "rusty-bacnet"]}
        for label, args in artifacts.items():
            out = cargo("tree", "--locked", "--offline", *args, "--target", target,
                        "-e", "normal,no-proc-macro", "--prefix", "none", "--format", "{p}")
            for crate in parse_tree(out):
                used.setdefault(crate, set()).add(label)
    return used


def index_packages(metadata):
    """{(name, version): package} from `cargo metadata`; refuses ambiguous pairs."""
    index = {}
    for pkg in metadata["packages"]:
        key = (pkg["name"], pkg["version"])
        if key in index:
            raise NoticesError(f"{key[0]} {key[1]} comes from two sources")
        index[key] = pkg
    return index


def normalize(text):
    lines = [line.rstrip() for line in text.replace("\r\n", "\n").split("\n")]
    return "\n".join(lines).strip("\n") + "\n"


def licence_comment(path):
    """The licence in a C file's leading /* ... */ comment, without the comment marks."""
    text = path.read_text(encoding="utf-8", errors="replace").replace("\r\n", "\n")
    start, end = text.find("/*"), text.find("*/")
    if text[:start].strip() or start < 0 or end < start:
        raise NoticesError(f"{path} doesn't start with a comment")
    lines = [re.sub(r"^\s*\*( |$)", "", line) for line in text[start + 2:end].split("\n")]
    comment = normalize("\n".join(lines))
    if "Copyright" not in comment or "Redistribution and use" not in comment:
        raise NoticesError(f"{path}'s leading comment is no longer its licence")
    return comment


def license_files(pkg):
    """[(file name as shown, text)] for the licence files a package ships."""
    root = Path(pkg["manifest_path"]).parent
    paths = sorted(p for p in root.iterdir()
                   if p.is_file() and LICENSE_FILE.match(p.name) and p.suffix.lower() not in SOURCE_SUFFIXES)
    if pkg.get("license_file"):
        declared = Path(os.path.normpath(root / pkg["license_file"]))
        if declared.is_file() and not any(declared.samefile(p) for p in paths):
            paths.append(declared)
    found = [(str(p.relative_to(root)) if p.is_relative_to(root) else p.name,
              normalize(p.read_text(encoding="utf-8", errors="replace"))) for p in paths]
    for extra, kind, covers in EXTRA_LICENSES.get(pkg["name"], []):
        path = root / extra
        if not path.is_file():
            raise NoticesError(f"{pkg['name']} {pkg['version']} no longer has {extra}; update EXTRA_LICENSES")
        text = licence_comment(path) if kind == "comment" else normalize(
            path.read_text(encoding="utf-8", errors="replace"))
        found.append((f"{extra}: {covers}" if covers else extra, text))
    if pkg["name"] in VENDORED_LICENSES:
        version, name, origin = VENDORED_LICENSES[pkg["name"]]
        if pkg["version"] != version:
            raise NoticesError(f"{pkg['name']} is {pkg['version']}, but its vendored licence is for {version};"
                               " check the new release's licence and update VENDORED_LICENSES")
        path = VENDORED_DIR / name
        if not path.is_file():
            raise NoticesError(f"the vendored licence {name} for {pkg['name']} is missing")
        found.append((f"{name}: {origin}", normalize(path.read_text(encoding="utf-8"))))
    return found


def needs_notice(expression):
    """Whether a licence expression may need the notice to go with a binary:
    true unless every identifier in it is in NO_NOTICE. No licence at all
    counts too. Deliberately strict: "MIT OR Unlicense" counts."""
    if not expression:
        return True
    words = SPDX_WORD.findall(expression.replace("/", " OR "))
    ids, skip = [], False
    for word in words:
        if word in ("AND", "OR"):
            continue
        if word == "WITH":
            skip = True  # the exception that follows isn't a licence
            continue
        if not skip:
            ids.append(word)
        skip = False
    return any(i not in NO_NOTICE for i in ids)


def check_license_files(crates):
    """Errors for crates that need a licence notice but ship no licence file."""
    errors = []
    for name, ver, lic, _source, _used, files in crates:
        if not files and needs_notice(lic) and name not in ALLOW_NO_LICENSE_FILE:
            errors.append(f"{name} {ver} ({lic or 'no licence declared'}) ships no licence file; find its"
                          " licence text and add it with EXTRA_LICENSES, or add the crate to"
                          " ALLOW_NO_LICENSE_FILE with the reason it needs none")
    return errors


def source_url(pkg):
    """Where to get a crate's source: its crates.io page for that version, or
    its repository if it doesn't come from crates.io."""
    if pkg.get("source") in CRATES_IO:
        return f"https://crates.io/crates/{pkg['name']}/{pkg['version']}"
    if pkg.get("repository"):
        return pkg["repository"]
    raise NoticesError(f"{pkg['name']} {pkg['version']} isn't from crates.io and declares no repository,"
                       " so the notices can't say where its source is")


def libpcap_source(version):
    return f"https://www.tcpdump.org/release/libpcap-{version}.tar.xz"


def render(version, own_license, crates, libpcap):
    """The notices text.

    crates: [(name, version, licence expression, source URL, {labels}, [(file, text)])];
    libpcap: (version, licence text) or None.
    """
    labels = {"CLI": "CLI", "Python": "Python"}
    out = [
        "THIRD-PARTY NOTICES", RULE, "",
        f"Rusty BACnet {version} release binaries:",
        "- CLI: the bacnet command-line tool (bacnet-linux-amd64, bacnet-linux-arm64,",
        "  bacnet-macos-amd64, bacnet-macos-arm64, bacnet-windows-amd64.exe), built",
        "  with the sc-tls feature, and on Linux also with the pcap feature;",
        "- Python: the rusty_bacnet extension module in the wheels for Linux, macOS",
        "  and Windows.",
        "",
        "Some components are only in the binaries for some platforms; the list",
        "below covers every platform.",
        "",
        "Rusty BACnet itself is under the MIT licence, below. The binaries also",
        "contain the third-party software listed after it, which is under the",
        "licences that follow the list. Each component is listed with where its",
        "source code can be obtained: for a crate, its crates.io page for the",
        "version compiled in, which offers that version's source package.",
        "", RULE, "Rusty BACnet", RULE, "", own_license.rstrip("\n"), "",
        RULE, "Components (licence as declared, the binaries that contain it, and the source)", RULE, "",
    ]
    rows = [(n, v, lic or "(none declared)", " ".join(labels[x] for x in sorted(used)), source)
            for n, v, lic, source, used, _files in crates]
    if libpcap:
        rows.append(("libpcap", libpcap[0], "BSD-3-Clause", "CLI", libpcap_source(libpcap[0])))
    for name, ver, lic, used, source in sorted(rows):
        out += [f"{name} {ver}: {lic} [{used}]", f"  source: {source}"]
    if libpcap:
        out += ["", RULE, f"libpcap {libpcap[0]} (https://www.tcpdump.org/), linked into the Linux CLI", RULE, "",
                libpcap[1].rstrip("\n")]

    groups = {}
    missing = []
    for name, ver, lic, _source, _used, files in crates:
        if not files:
            why = ALLOW_NO_LICENSE_FILE.get(name) or (
                "" if needs_notice(lic) else "its licence doesn't ask for the notice in a binary")
            missing.append(f"{name} {ver}: {lic or '(none declared)'}; {why}")
        for file_name, text in files:
            groups.setdefault(text, []).append(f"{name} {ver} ({file_name})")
    out += ["", RULE, "Licence texts, each followed by the crates that ship it", RULE]
    for text, users in sorted(groups.items(), key=lambda item: sorted(item[1])[0]):
        out += ["", "-" * 79, *[f"  {u}" for u in sorted(users)], "-" * 79, "", text.rstrip("\n")]
    if missing:
        out += ["", RULE, "Crates whose package has no licence file (licence as declared, and why none is needed)",
                RULE, "", *missing]
    return "\n".join(out) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--license", default="LICENSE", type=Path, help="Rusty BACnet's own licence")
    parser.add_argument("--libpcap", type=Path, help="directory with libpcap's LICENSE and VERSION")
    args = parser.parse_args(argv)
    try:
        metadata = json.loads(cargo("metadata", "--format-version", "1", "--locked", "--offline", "--all-features"))
        index = index_packages(metadata)
        members = set(metadata["workspace_members"])
        version = next(p["version"] for p in metadata["packages"] if p["name"] == "rusty-bacnet")
        crates = []
        for key, used in sorted(linked_crates().items()):
            pkg = index.get(key)
            if pkg is None:
                raise NoticesError(f"cargo tree lists {key[0]} {key[1]}, which cargo metadata doesn't")
            if pkg["id"] in members:
                continue
            crates.append((pkg["name"], pkg["version"], pkg.get("license"), source_url(pkg), used,
                           license_files(pkg)))
        errors = check_license_files(crates)
        if errors:
            raise NoticesError("\n".join(errors))
        libpcap = None
        if args.libpcap:
            libpcap = ((args.libpcap / "VERSION").read_text().strip(),
                       normalize((args.libpcap / "LICENSE").read_text(encoding="utf-8")))
        text = render(version, normalize(args.license.read_text(encoding="utf-8")), crates, libpcap)
    except (NoticesError, OSError, subprocess.CalledProcessError) as err:
        detail = getattr(err, "stderr", "") or ""
        print(f"error: {err}\n{detail}".rstrip(), file=sys.stderr)
        return 1
    args.out.write_text(text, encoding="utf-8")
    without = sum(1 for c in crates if not c[5])
    print(f"wrote {args.out}: {len(crates)} crates{', libpcap' if libpcap else ''}, {len(text)} characters;"
          f" {without} crates ship no licence file")
    return 0


if __name__ == "__main__":
    sys.exit(main())
