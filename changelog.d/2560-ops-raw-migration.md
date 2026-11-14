---
section: Migration notes
---
- **bacnet-ops raw layer (Rust API, #2560):** import `Reader`, `Halt`,
  `RpmFailure`, `rpm_failure`, `rpm_rows`, `unreachable` and `timed_out`
  from `bacnet_ops::raw`; match `ReadVia::Mixed`; `MAX_BATCH` and
  `MAX_BATCH_PROPERTIES` are gone; `DeviceSummary` is only `PartialEq`;
  give `read_event_information` a fetch whose error is `From<PagingFault>`;
  `bip6_vmac` fails with `VmacLookup`.
