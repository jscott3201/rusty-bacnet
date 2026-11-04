# Counting harness: compare run

- A: `dd786122c4eea1c77ef0ea0d2a7dea2720a9e12b` (dd786122c4eea1c77ef0ea0d2a7dea2720a9e12b); binaries codec-counts `40c7b74d12162b91`, counts `7f8cff6b5834bb4e`
- A': `dd786122c4eea1c77ef0ea0d2a7dea2720a9e12b` (dd786122c4eea1c77ef0ea0d2a7dea2720a9e12b); binaries codec-counts `40c7b74d12162b91`, counts `7f8cff6b5834bb4e`
- B: `d6de4282e5b2db43f70ef8b3cd4abd861a61a404` (d6de4282e5b2db43f70ef8b3cd4abd861a61a404); binaries codec-counts `40c7b74d12162b91`, counts `38585ac96ff0bad6`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 31,266.5 | 31,026.3 | -240.2 | 12.9 (A vs A' in this matrix) | -18.7 x | 6 of 6 | 18.0005 / 18.0005 | 2127.05 / 2127.05 | PASS |
| rp | mt | 32,643.9 | 32,413.0 | -230.8 | 41.7 (A vs A' in this matrix) | -5.5 x | 6 of 6 | 18.0008 / 18.0008 | 2127.08 / 2127.08 | PASS |
| rpm | ct | 91,137.3 | 90,894.6 | -242.7 | 12.5 (A vs A' in this matrix) | -19.4 x | 6 of 6 | 139.0011 / 139.0011 | 18496.12 / 18496.12 | PASS |
| rpm | mt | 94,216.0 | 94,061.6 | -154.4 | 480.5 (A vs A' in this matrix) | -0.3 x | 6 of 6 | 139.0015 / 139.0014 | 18496.16 / 18496.15 | PASS |
| wp | ct | 39,900.2 | 39,657.1 | -243.1 | 98.8 (A vs A' in this matrix) | -2.5 x | 6 of 6 | 25.0006 / 25.0006 | 7944.06 / 7944.06 | PASS |
| wp | mt | 41,171.6 | 40,890.4 | -281.2 | 141.5 (A vs A' in this matrix) | -2.0 x | 6 of 6 | 25.0009 / 25.0009 | 7944.09 / 7944.09 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: rp. Accepted: True
