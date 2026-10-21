#!/usr/bin/env python3
"""The release version: the one version the publishable crates share (#1768).

    cargo metadata --no-deps --format-version 1 --locked | release_version.py

Reads `cargo metadata` on stdin and prints the release version, which the
tag must name (v<version>) and the Python package takes from
crates/rusty-bacnet. Every publishable crate (`publish` not `[]`) carries it,
except the crates in INDEPENDENT: they version on their own, taking a new
version only when their own API changes, so theirs may differ.

Fails, naming the crates, when the others disagree, and when a crate in
INDEPENDENT isn't a publishable member of the workspace, so the list can't
outlive its crates.
"""

import json
import sys

# Crates that version on their own (#1768). Their manifests set `version`
# themselves instead of inheriting the workspace's; test_release_version.py
# checks the two agree.
INDEPENDENT = ("bacnet-time",)


class VersionError(Exception):
    """The workspace has no single release version."""


def release_version(metadata, independent=INDEPENDENT):
    """The version every publishable crate outside `independent` carries."""
    publishable = {package["name"]: package["version"]
                   for package in metadata["packages"] if package.get("publish") != []}
    if not publishable:
        raise VersionError("cargo metadata lists no publishable crates")
    missing = sorted(set(independent) - set(publishable))
    if missing:
        raise VersionError(f"independently versioned crates that aren't publishable members: {', '.join(missing)}")
    versions = {}
    for name, version in publishable.items():
        if name not in independent:
            versions.setdefault(version, []).append(name)
    if len(versions) != 1:
        shown = "; ".join(f"{version}: {', '.join(sorted(names))}" for version, names in sorted(versions.items()))
        raise VersionError(f"publishable crates disagree: {shown}")
    return next(iter(versions))


def main():
    try:
        print(release_version(json.load(sys.stdin)))
    except (VersionError, KeyError, json.JSONDecodeError) as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
