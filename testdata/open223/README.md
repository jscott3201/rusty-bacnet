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
  the Apache License 2.0, not under this repository's MIT licence; the
  licence's text is in [`LICENSE`](LICENSE) beside it.
- **SHA-256:** `08f80bd6e15052f28cdfd354bdf46ff96e5e1afdba36a4dbed5186fc1ea51c41`

The file is kept unmodified. It sits outside every crate directory, so no
published crate ships it. Three repository tools read this directory:
`scripts/check-223.py`, which loads the ontology; one `bacnet-ops` unit test,
which searches the file's text for the namespaces the export writes (see
step 3 below); and the no-secret scan (`scripts/ci/check-no-secrets.sh`),
which greps every tracked file. No other check reads it. The ASHRAE 135 overlap check (`spec_overlap.py`) is run on
diffs that leave this directory out, by convention: the file is third-party
text kept verbatim.

## Refreshing it

1. Fetch the file, as plain HTTP with no identifying details:
   `curl -sS -o testdata/open223/223p.ttl https://open223.info/223p.ttl`.
2. Check that its header still declares Apache-2.0 (`grep -n dcterms:license`).
   If it doesn't, don't commit it: ask the maintainer.
3. Check the vocabulary namespace the external-reference shape
   (`s223:BACnetExternalReferenceShape`) uses for its `bacnet:` paths:
   `grep -n '^@prefix bacnet:' testdata/open223/223p.ttl`. This copy has
   `http://data.ashrae.org/bacnet/2020#`. If it changed, update the
   `bacnet` entry of `PREFIXES` in
   `crates/bacnet-ops/src/export/s223/turtle.rs` in the same change, and
   re-bless the golden files. The test
   `vendored_ontology_declares_the_written_namespaces` in `bacnet-ops`
   fails until the two agree.
4. Update the fetch date, the version string and the SHA-256 above, and
   `ONTOLOGY_VERSION` in `crates/bacnet-ops/src/export/s223/mod.rs`, which
   the export's header comment names.
5. Run `python3 scripts/check-223.py` and fix the export or its golden files
   for anything the new shapes refuse.
