---
section: Migration notes
---
- **Frame summaries (Rust API, #2614):** read `FrameSummary::link_message`
  where you read `link`. A `FrameSummary` literal or pattern names the new
  `original_source` and `parse_error` fields (or ends in `..`), a `Link`
  literal names `original_source`, and a `match` on `Framing` needs a
  wildcard arm.
