# Counting harness: compare run

- A: `dd786122c4eea1c77ef0ea0d2a7dea2720a9e12b` (dd786122c); binaries codec-counts `40c7b74d12162b91`, counts `7f8cff6b5834bb4e`
- A': `dd786122c4eea1c77ef0ea0d2a7dea2720a9e12b` (dd786122c); binaries codec-counts `40c7b74d12162b91`, counts `7f8cff6b5834bb4e`
- B: `d6de4282e5b2db43f70ef8b3cd4abd861a61a404` (d6de4282e); binaries codec-counts `40c7b74d12162b91`, counts `38585ac96ff0bad6`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 31,253.9 | 31,011.0 | -242.9 | 14.2 (A vs A' in this matrix) | -17.1 x | 6 of 6 | 18.0001 / 18.0001 | 2127.01 / 2127.02 | PASS |
| rp | mt | 32,554.6 | 32,300.9 | -253.7 | 101.5 (A vs A' in this matrix) | -2.5 x | 6 of 6 | 18.0003 / 18.0003 | 2127.03 / 2127.03 | FAIL |
| rpm | ct | 91,106.5 | 90,864.6 | -241.9 | 42.0 (A vs A' in this matrix) | -5.8 x | 6 of 6 | 139.0004 / 139.0004 | 18496.04 / 18496.04 | PASS |
| rpm | mt | 93,102.9 | 92,948.1 | -154.8 | 985.4 (A vs A' in this matrix) | -0.2 x | 5 of 6 | 139.0005 / 139.0004 | 18496.06 / 18496.04 | PASS |
| wp | ct | 39,881.7 | 39,641.6 | -240.1 | 98.8 (A vs A' in this matrix) | -2.4 x | 6 of 6 | 25.0002 / 25.0002 | 7944.02 / 7944.02 | PASS |
| wp | mt | 41,076.5 | 40,836.6 | -239.9 | 186.3 (A vs A' in this matrix) | -1.3 x | 6 of 6 | 25.0003 / 25.0003 | 7944.03 / 7944.03 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: rp. Accepted: False
