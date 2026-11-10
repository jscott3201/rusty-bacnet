# Counting harness: compare run

- A: `9e6bb098106239db36c2094916775ad8502b12ca` (9e6bb0981); binaries codec-counts `00a63321146442a8`, counts `5f98dcd568265b9d`
- A': `9e6bb098106239db36c2094916775ad8502b12ca` (9e6bb0981); binaries codec-counts `00a63321146442a8`, counts `5f98dcd568265b9d`
- B: `493b8647b279b172c57c3befd3cc2334dad84f3f` (493b8647b); binaries codec-counts `d2ce0b8743eba813`, counts `67f5488b072e1cb3`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 31,008.8 | 31,009.0 | 0.2 | 15.1 (A vs A' in this matrix) | +0.0 x | 2 of 6 | 18.0001 / 18.0001 | 2127.01 / 2127.01 | PASS |
| rp | mt | 32,305.3 | 32,302.3 | -2.9 | 169.1 (A vs A' in this matrix) | -0.0 x | 4 of 6 | 18.0003 / 18.0003 | 2127.03 / 2127.03 | PASS |
| rpm | ct | 90,438.8 | 90,435.5 | -3.3 | 83.5 (A vs A' in this matrix) | -0.0 x | 4 of 6 | 139.0004 / 139.0004 | 19000.04 / 19000.04 | PASS |
| rpm | mt | 92,950.6 | 93,059.9 | 109.4 | 689.1 (A vs A' in this matrix) | +0.2 x | 2 of 6 | 139.0005 / 139.0005 | 19000.06 / 19000.06 | PASS |
| wp | ct | 39,918.7 | 39,918.3 | -0.4 | 213.6 (A vs A' in this matrix) | -0.0 x | 3 of 6 | 25.0002 / 25.0002 | 8448.02 / 8448.02 | PASS |
| wp | mt | 41,114.7 | 41,133.2 | 18.5 | 247.8 (A vs A' in this matrix) | +0.1 x | 2 of 6 | 25.0003 / 25.0002 | 8448.04 / 8448.02 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: none. Accepted: True
