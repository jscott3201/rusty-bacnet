# Draft BACnet BIBB Support Evidence

> DRAFT internal support evidence. Generated from `conformance/bacnet-135-2020.json`; this is not a BTL certification claim or formal PICS/BIBB declaration.

This draft is a ledger-derived starting point for future Annex K BIBB mapping. Detailed BIBB claims require service-specific positive and negative tests.

## P1

| ID | Anchor | Status | Tests |
|---|---|---|---|
| `BACNET-12-OBJECT-MODEL` | Clauses 12-19 | implementation-present-needs-conformance-tests | `crates/bacnet-server/src/pics/tests.rs`, `crates/bacnet-objects/src`, `crates/bacnet-server/src/handlers/tests` |
| `BACNET-21-FORMAL-APDUS` | Clause 21 | implementation-present-needs-conformance-tests | - |
| `BACNET-K-BIBBS` | Annex K | in-progress | - |
