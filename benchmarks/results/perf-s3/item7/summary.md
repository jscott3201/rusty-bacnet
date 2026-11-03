# Counting harness: compare run

- A: `123414b3b45ad139d2c6a73f2f89800873b49e67` (123414b3b); binaries codec-counts `1e45abba58cd5add`, counts `6e6c4e726d616b7a`
- A': `123414b3b45ad139d2c6a73f2f89800873b49e67` (123414b3b); binaries codec-counts `1e45abba58cd5add`, counts `6e6c4e726d616b7a`
- B: `8706ea6b8c9f99f01958162c60e51073f6c424fd` (8706ea6b8); binaries codec-counts `1e45abba58cd5add`, counts `d74026f06e59d776`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 33,311.5 | 33,230.6 | -80.9 | 13.1 (A vs A' in this matrix) | -6.2 x | 6 of 6 | 22.0001 / 22.0001 | 2196.01 / 2196.01 | PASS |
| rp | mt | 34,623.9 | 34,543.1 | -80.9 | 36.4 (A vs A' in this matrix) | -2.2 x | 6 of 6 | 22.0002 / 22.0002 | 2196.02 / 2196.02 | PASS |
| rpm | ct | 92,349.7 | 92,305.8 | -43.9 | 11.0 (A vs A' in this matrix) | -4.0 x | 3 of 6 | 141.0005 / 141.0005 | 18797.05 / 18797.05 | PASS |
| rpm | mt | 94,658.2 | 94,628.0 | -30.1 | 507.5 (A vs A' in this matrix) | -0.1 x | 2 of 6 | 141.0004 / 141.0004 | 18797.04 / 18797.04 | PASS |
| wp | ct | 42,135.2 | 42,048.8 | -86.4 | 209.5 (A vs A' in this matrix) | -0.4 x | 6 of 6 | 27.0002 / 27.0002 | 8011.02 / 8011.02 | PASS |
| wp | mt | 43,376.8 | 43,290.8 | -85.9 | 145.6 (A vs A' in this matrix) | -0.6 x | 6 of 6 | 27.0004 / 27.0003 | 8011.04 / 8011.04 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: none. Accepted: True
