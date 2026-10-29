# Design: conformance and support reporting on the website

Status: **decided** (maintainer, 2026-10-25): option C in section 4, in two
PRs, tables first; no BIBBs, no ledger. This settles the project's open
decision OD-07 ("how to present conformance", #1496, folded into #1989). The
options stay below for context. Both PRs are built: the first (part B: the
catalogue, the client-sent list, the generator, `support.json` and
`project/conformance`), then the second (#2129): the sample PICS
(`project/sample-pics`, written by the same `support-tables` example of
`bacnet-integration-tests`) and the `integrate/pics` task page. Section 2
describes the sources as they were before it. One finding from that PR: the
server sends its requests through its own outbound paths, never through
`bacnet-client`, so the client-sent list can't vouch for the PICS initiator
list. Instead a test scans the server's source for the request PDUs it builds
and ties `INITIATED_SERVICES` to them; that added audit notifications,
WriteProperty, ReadProperty and Who-Is, which the server sends only when
configured. Spec references are clause numbers only.

## 1. Problem

Integrators deciding whether the stack fits, and anyone preparing a PICS or a
BTL submission, need to know which object types, services and data links each
surface supports, and what is known to be missing. Today they get one
hand-written section, "What's supported" on `project/support`, which, for v0.14.0,
names 64 object types, 25 confirmed services the server executes and 24
services the client sends (counted from that page's "Full server" and
"Client" paragraphs), the data links and the Python and CLI surfaces. It is accurate for v0.14.0, but every count and
list in it is typed by hand, so it goes stale the first time a PR adds a
service without touching the page.

The conformance ledger removed in #1497 failed the other way: a hand-kept row
per clause, with spec-derived prose, that every PR had to edit and few people
read. This note does not bring it back. Every option below generates from code
or stays as small as today's summary.

## 2. Sources of truth in the code

| Fact | Source | Already guarded by |
|---|---|---|
| Object types implemented | One `BACnetObject` implementation per type; `supported_representatives()` in `bacnet-objects/src/property_metadata_audit.rs` builds one of each (64; Network Security excluded) | That audit test, which fails if a type falls back to default metadata |
| Per-type properties, access, creation-only rows | Each object's `property_metadata`, read by `bacnet_server::pics` | PICS truth-source and property-union tests |
| Services the full server executes | `bacnet_objects::device::EXECUTED_SERVICES` | `executed_services_match_dispatch_table` in `bacnet-server/src/pics/tests.rs` |
| Services the server initiates | `PicsGenerator::INITIATED_SERVICES` (private) | None against the client crate |
| Services the client sends | `BACnetClient`'s public methods | None as a list |
| Shared-endpoint responder | `bacnet-endpoint` server-role declaration (ReadProperty, opt-in WriteProperty) | Endpoint tests of the advertised services |
| Data links | `pics::DataLinkSupport`; Cargo features of each artifact (CLI release build, Python manifest) | The website release test pins the release artifact list |
| Python surface | `crates/rusty-bacnet/rusty_bacnet.pyi` | Stub parity tests in the Python suite (endpoints, enums) |
| A configured device's PICS | `BACnetServer::generate_pics`, `bacnet_server::pics::generate_pics`, rendered by `Pics` as text or Markdown | PICS unit tests |

Two gaps stand out: the representative list is test-only, and nothing ties the
initiated-service list to what the client crate can send. Python has no PICS
entry point.

## 3. Questions and options

### 3.1 Level of detail

The levels are labelled L1 (least detail) to L4 (most).

| Level | Content | Readers served | Cost |
|---|---|---|---|
| L1 Summary | Today's prose section | Evaluators skimming | Drifts unless tested |
| L2 Capability tables | Object types (with create/delete support); services by surface (full server executes and initiates, client sends, endpoint responder, Python); data links by artifact (Rust feature, Python wheel, CLI release binary) | Integrators comparing stacks | Small if generated |
| L3 Sample PICS | The generator's Markdown for one reference device configuration, labelled as an example of a configuration, not a claim | People preparing a PICS | One generated file |
| L4 Per-property matrix | Every property of every type with access flags | Few readers | Large, close to retelling Clause 12 |

BIBBs are deliberately absent from every level. A BIBB claim (Annex K) is a
judgement about behaviour that BTL tests; deriving one from a service list would
overclaim. If BIBBs appear later, they need a reviewed mapping and tests, not
generation.

### 3.2 Generated versus hand-written

Three mechanisms: H keeps the page hand-written, G1 commits generated files,
G2 generates during the site build.

- **H. Hand-written, with a guard.** Keep the prose, and add a Rust test that
  reads the support page and fails when its counts disagree with the code.
  Cheap, but the test has to parse prose and the lists still need hand edits.
- **G1. Generated data, committed.** A small generator (a Rust example or
  integration-test helper) builds a reference database from the
  representatives, calls the PICS generator and the service constants, and
  writes `website/src/data/support.json` and `website/src/content/…/sample-pics.md`.
  A Rust test regenerates both in memory and fails if the committed files
  differ, in the same way the website sample tests (#2010, #2014) fail when a
  guide's source drifts. The website renders tables from the JSON. No new CI:
  the check runs in the existing workspace tests.
- **G2. Generated at site build.** The Docs job would need a Rust toolchain
  and a build of `bacnet-server`. Slower, and the Docs job is not part of the
  merge gate, so drift would be caught later. Not recommended.

### 3.3 Sample PICS

Publish one generated PICS for a documented reference configuration (every
supported object type once, B/IP and SC data links, router and BBMD flags off),
with a banner saying it describes that configuration only, and a task page
(`integrate/pics`) showing how to generate a PICS for your own device. The
generator's output already carries clause numbers rather than spec text.

### 3.4 Gaps

Do not keep a list of gaps in the page or the generator. Label open issues that
describe a missing object, service or data-link behaviour (for example
`conformance-gap`) and link the page to that label's issue search. The page
stays current without edits, and each gap has a place for evidence and
discussion.

### 3.5 Keeping claims honest

- One source per fact (section 2). No hand-typed counts on any page.
- The generated files carry the release they describe; the release PR
  regenerates them, and the website release test pins that version.
- Each table states its surface: a type the full server serves is not a type
  the endpoint responder serves, and a Rust feature is not a Python or CLI
  capability.
- Pages cite clause numbers only, write no spec prose and hold no history.
- New tests close the two gaps: the representative list becomes a public,
  documented function (or a `#[doc(hidden)]` one behind a feature), and a test
  ties `INITIATED_SERVICES` and a client-sent list to the client's methods.
- **A sample PICS can be copied as a device's claim.** It carries a banner
  and a title naming the reference configuration, says it is generated
  output for that configuration only, and is never offered as a download
  without the banner.
- **"Supported object types" counts constructible types, not tested
  behaviour.** The tables say so in their heading, and link each surface to
  the evidence guidance on `project/support`.
- Generated tables and pages never use the words "conformant", "compliant" or
  "certified".

## 4. Options (decision OD-07)

| Option | Detail | Generated | Effort | Staleness risk |
|---|---|---|---|---|
| **A** | L1 + guard test (H) | No | Small | Medium: prose edits still manual |
| **B** | L2 capability tables + gap label link (G1) | Yes | Medium: generator, JSON, one page | Low |
| **C** | B + L3 sample PICS + `integrate/pics` task page | Yes | Medium, plus one page | Low |
| **D** | C + L4 per-property matrix | Yes | Large | Low, but long and spec-adjacent |

**Decision: C, in two PRs** (the recommendation, accepted on 2026-10-25). First B: the generator, `support.json`, the
freshness test and a `project/conformance` page whose tables replace the lists
in "What's supported" (the support page keeps the evidence-reading guidance and
links to the new page). Then the sample PICS and the task page. This answers
both audiences and adds no workflow. It builds on the PICS generator, the
executed-service list and the dispatch cross-check, which exist and are
tested, but it needs two changes first: `supported_representatives()` is a
private function in a test-only module (`bacnet-objects/src/property_metadata_audit.rs:144`)
and must become a public, documented catalogue; and `INITIATED_SERVICES` is a
private constant (`bacnet-server/src/pics/mod.rs:523`) with no test tying it,
or a client-sent list, to what `bacnet-client` can send. Option A is the fallback if the maintainer wants no new page
before 0.15.0; D is not recommended.

## 5. Out of scope

- A Python `generate_pics`; worth a separate issue if Python users ask for it.
- BTL listing or certification language of any kind.
- Restoring any part of the removed ledger, its scripts or its CI steps.
