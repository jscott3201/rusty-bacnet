---
title: "Upgrade to v0.11.0"
description: "Review breaking changes before replacing a working integration."
---

Do not treat this pre-1.0 update as a drop-in replacement merely because the package name stayed the same. Read the release notes and changelog, then test the application behaviors you actually use.

## Before updating

Record your current package/binary version, enabled features, platform, transport, and important application workflows. Back up persisted Audit Log snapshots before attempting a migration. Keep a known-working deployment or environment available according to your operational policy.

## Update the selected interface

In a Python virtual environment:

```sh
python -m pip install --upgrade rusty-bacnet==0.11.0
```

For Rust, align the application's `bacnet-*` dependencies with `0.11.0`. For the CLI, install or download the intended v0.11.0 binary and confirm its version and required features.

## Review the affected behavior

The release calls out CLI alarm timestamp inputs; Rust event and recipient APIs; typed Audit/COV models; object constructor changes; corrected wire/property values; and withdrawn unsupported server/WASM surfaces.

Audit Log persistence has a new receipt-aware snapshot format documented in the Rust API guide. Do not invent a generic converter or assume old and new snapshots are interchangeable. Follow the documented format and verify restoration in a non-production environment.

## Verify more than import success

Exercise a known read, error handling, any routing or segmentation paths you depend on, subscription lifecycle, relevant server objects, and controlled cleanup. SC and MS/TP deployments need transport-specific validation, not only package-level tests.

Retest your application's output parsing where it depends on CLI JSON or typed Python/Rust models. Separate regressions from corrected behavior that an older integration may have depended on accidentally.

## Keep development separate

The repository's default development branch and its newest published release are different concepts. The website can be maintained from `dev` while its public guides describe a named release. New functionality should become release guidance only after its release and examples are verified.

Current-dev SC has stricter CA, operational credential, and device UUID requirements than v0.11.0. Review the [SC version boundary and engineering migration links](/rusty-bacnet/guides/bacnet-sc/) before testing a development build; these changes are not silently part of the release instructions.

## Sources and release scope

These instructions target **v0.11.0**. Source review is not a claim of hardware qualification.

[v0.11.0 release notes](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.11.0) · [Changelog](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/CHANGELOG.md) · [Audit persistence documentation](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/rust-api.md).
