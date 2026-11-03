# Counting harness: compare run

- A: `211f1b61661f6de1ebb8bef5779fa30e476d8078` (211f1b616); binaries codec-counts `59eebb5eb9d236f8`, counts `7254cbb99df9de65`
- A': `211f1b61661f6de1ebb8bef5779fa30e476d8078` (211f1b616); binaries codec-counts `59eebb5eb9d236f8`, counts `7254cbb99df9de65`
- B: `5602a5b7ee68acb414c9149d665250c9dc9c14ee` (5602a5b7e); binaries codec-counts `38564a1177fa4f5f`, counts `16c6eb1160b5fa30`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 33,835.3 | 33,835.5 | 0.2 | 5.6 (A vs A' in this matrix) | +0.0 x | 3 of 6 | 22.0002 / 22.0002 | 2196.02 / 2196.02 | PASS |
| rp | mt | 35,141.0 | 35,141.1 | 0.1 | 22.9 (A vs A' in this matrix) | +0.0 x | 3 of 6 | 22.0002 / 22.0002 | 2196.02 / 2196.02 | PASS |
| rpm | ct | 92,885.8 | 92,784.3 | -101.5 | 289.6 (A vs A' in this matrix) | -0.4 x | 5 of 6 | 141.0004 / 141.0004 | 18797.04 / 18797.04 | PASS |
| rpm | mt | 95,171.1 | 95,380.4 | 209.3 | 635.8 (A vs A' in this matrix) | +0.3 x | 2 of 6 | 141.0005 / 141.0004 | 18797.05 / 18797.04 | PASS |
| wp | ct | 42,549.8 | 42,044.5 | -505.4 | 198.7 (A vs A' in this matrix) | -2.5 x | 6 of 6 | 27.0002 / 27.0002 | 8011.02 / 8011.02 | FAIL |
| wp | mt | 43,777.3 | 43,277.9 | -499.3 | 104.1 (A vs A' in this matrix) | -4.8 x | 6 of 6 | 27.0002 / 27.0003 | 8011.02 / 8011.03 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: wp. Accepted: False
