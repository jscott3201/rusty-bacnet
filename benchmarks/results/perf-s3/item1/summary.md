# Counting harness: compare run

- A: `a1d54afb8246e2b062d6bd5d408390c6d70ee02b` (a1d54afb8); binaries codec-counts `c87c02a3cd0826a3`, counts `a8282d44a0b6574d`
- A': `a1d54afb8246e2b062d6bd5d408390c6d70ee02b` (a1d54afb8); binaries codec-counts `c87c02a3cd0826a3`, counts `a8282d44a0b6574d`
- B: `123414b3b45ad139d2c6a73f2f89800873b49e67` (123414b3b); binaries codec-counts `1e45abba58cd5add`, counts `6e6c4e726d616b7a`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 33,826.1 | 33,312.4 | -513.8 | 11.8 (A vs A' in this matrix) | -43.6 x | 6 of 6 | 22.0001 / 22.0001 | 2196.02 / 2196.01 | PASS |
| rp | mt | 35,138.7 | 34,618.1 | -520.6 | 40.4 (A vs A' in this matrix) | -12.9 x | 6 of 6 | 22.0002 / 22.0002 | 2196.02 / 2196.02 | PASS |
| rpm | ct | 92,861.4 | 92,350.3 | -511.0 | 39.0 (A vs A' in this matrix) | -13.1 x | 6 of 6 | 141.0004 / 141.0004 | 18797.04 / 18797.04 | PASS |
| rpm | mt | 95,296.1 | 94,685.2 | -610.9 | 983.6 (A vs A' in this matrix) | -0.6 x | 6 of 6 | 141.0007 / 141.0007 | 18797.08 / 18797.07 | PASS |
| wp | ct | 42,653.3 | 42,137.4 | -515.8 | 114.0 (A vs A' in this matrix) | -4.5 x | 6 of 6 | 27.0003 / 27.0003 | 8011.03 / 8011.03 | PASS |
| wp | mt | 43,888.9 | 43,402.5 | -486.4 | 148.6 (A vs A' in this matrix) | -3.3 x | 6 of 6 | 27.0003 / 27.0003 | 8011.04 / 8011.04 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: rp. Accepted: True
