# Open223 ontology copy

`223p.ttl` is a verbatim copy of the Open223 build of the proposed ASHRAE
Standard 223 ontology. `scripts/check-223.py` validates the
`bacnet export 223-experimental` golden files
(`crates/bacnet-ops/tests/s223/`) against its SHACL shapes, with no
network fetch (#2436).

- **Source:** <https://open223.info/223p.ttl>
- **Fetched:** 2026-11-10 (HTTP `Last-Modified: Fri, 16 Oct 2026 15:38:15 GMT`)
- **Version:** `owl:versionInfo "git:master@6fdc11c60b930ccb042d775b66a9115547fd22a7 date:2026-10-06 20:01:58 UTC"`
- **Licence:** the file's ontology header declares
  `dcterms:license <http://www.apache.org/licenses/LICENSE-2.0>` and
  `dcterms:rights "Copyright 2026 ASHRAE"`. It is third-party text under
  the Apache License 2.0, not under this repository's MIT licence.
- **SHA-256:** `08f80bd6e15052f28cdfd354bdf46ff96e5e1afdba36a4dbed5186fc1ea51c41`

The file is kept unmodified. It sits outside every crate directory, so no
published crate ships it, and the repository's own text checks (spec
overlap, wording) leave it out.

## Refreshing it

1. Fetch the file, as plain HTTP with no identifying details:
   `curl -sS -o testdata/open223/223p.ttl https://open223.info/223p.ttl`.
2. Check that its header still declares Apache-2.0 (`grep -n dcterms:license`).
   If it doesn't, don't commit it: ask the maintainer.
3. Update the fetch date, the version string and the SHA-256 above, and
   `ONTOLOGY_VERSION` in `crates/bacnet-ops/src/export/s223/mod.rs`, which
   the export's header comment names.
4. Run `python3 scripts/check-223.py` and fix the export or its golden files
   for anything the new shapes refuse.
