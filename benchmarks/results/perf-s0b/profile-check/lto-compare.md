# Counting harness: compare run

- A: `64bb7eb27f0b27657f4591b2c9a5dfc06efaf097` (64bb7eb27); binaries codec-counts `1e45abba58cd5add`, counts `4aaabb3458c8b305`
- A': `64bb7eb27f0b27657f4591b2c9a5dfc06efaf097` (64bb7eb27); binaries codec-counts `1e45abba58cd5add`, counts `4aaabb3458c8b305`
- B: `3a2bd4b73d353f43121590df376a5f0deadce7e1` (3a2bd4b73); binaries codec-counts `c87c02a3cd0826a3`, counts `fe698e3c426dd481`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 33,826.9 | 33,825.4 | -1.5 | 13.3 (A vs A' in this matrix) | -0.1 x | 4 of 6 | 22.0001 / 22.0001 | 2196.01 / 2196.01 | PASS |
| rp | mt | 35,173.4 | 35,169.2 | -4.2 | 84.7 (A vs A' in this matrix) | -0.0 x | 2 of 6 | 22.0002 / 22.0002 | 2196.02 / 2196.02 | PASS |
| rpm | ct | 98,534.9 | 99,100.6 | 565.8 | 1,404.5 (A vs A' in this matrix) | +0.4 x | 1 of 6 | 141.0004 / 141.0004 | 18797.04 / 18797.04 | PASS |
| rpm | mt | 101,805.6 | 101,817.4 | 11.8 | 627.5 (A vs A' in this matrix) | +0.0 x | 3 of 6 | 141.0005 / 141.0005 | 18797.05 / 18797.05 | PASS |
| wp | ct | 42,691.6 | 42,691.9 | 0.2 | 23.8 (A vs A' in this matrix) | +0.0 x | 3 of 6 | 27.0002 / 27.0002 | 8011.02 / 8011.02 | PASS |
| wp | mt | 43,901.9 | 43,898.3 | -3.6 | 243.7 (A vs A' in this matrix) | -0.0 x | 3 of 6 | 27.0002 / 27.0002 | 8011.02 / 8011.02 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: none. Accepted: True
