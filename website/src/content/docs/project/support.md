---
title: "Support status and evidence"
description: "Understand pre-1.0 release scope without mistaking tests for certification."
---

Rusty BACnet v0.11.0 is a **pre-1.0 release with breaking APIs and partial conformance coverage**. The release makes no BTL certification or full BACnet conformance claim.

## What the site can tell you

Guides identify the release, interface, configuration prerequisites, and known boundaries behind a task. They should make it easier to perform a small operation correctly and to recognize when evidence is missing.

## What the site cannot establish

A screenshot, successful build, unit-test count, or single-device demonstration does not establish all-device interoperability, deterministic serial timing, complete object/service coverage, or fitness for a particular operating building.

The conformance ledger contains scoped evidence and incomplete-review labels. Its machine-readable source is `docs/conformance/bacnet-135-2020.json`. A future site view may render that source, but must retain its exact status meaning and explicitly handle missing or unrecognized status values.

## Read evidence at the right scope

“Implementation present” is different from “tested for this clause.” A clause-evidenced property rule is different from a fully qualified object. A standalone transport example is different from a combined endpoint profile. A draft PICS is different from a formal declaration or BTL listing.

Do not turn those distinctions into one green “supported” checkmark. This site deliberately uses explanatory text alongside any visual status indicators.

## Report a problem

Start with [troubleshooting](/rusty-bacnet/help/troubleshooting/). Provide the release/build, transport, platform, minimal reproduction, and sanitized evidence. Include the exact source of a conflicting documentation claim so it can be fixed along with the code or example.

The documentation's reviewed baseline is v0.11.0. Later development work is not silently treated as released capability. Consult release notes when upgrading.

## Sources and release scope

These instructions target **v0.11.0**. Source review is not a claim of hardware qualification.

[Release scope](https://github.com/jscott3201/rusty-bacnet/releases/tag/v0.11.0) · [Conformance ledger](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/conformance/standard-135-2020-ledger.md) · [Machine-readable evidence](https://github.com/jscott3201/rusty-bacnet/blob/v0.11.0/docs/conformance/bacnet-135-2020.json).
