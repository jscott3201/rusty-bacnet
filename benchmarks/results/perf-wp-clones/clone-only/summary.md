# Counting harness: compare run

- A: `211f1b61661f6de1ebb8bef5779fa30e476d8078` (211f1b616); binaries codec-counts `59eebb5eb9d236f8`, counts `7254cbb99df9de65`
- A': `211f1b61661f6de1ebb8bef5779fa30e476d8078` (211f1b616); binaries codec-counts `59eebb5eb9d236f8`, counts `7254cbb99df9de65`
- B: `0ea3c14db24e7d07438f89c39db52d49b4c3373f` (0ea3c14db); binaries codec-counts `38564a1177fa4f5f`, counts `e25b2d9e79719b16`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 33,833.1 | 33,835.5 | 2.4 | 11.7 (A vs A' in this matrix) | +0.2 x | 1 of 6 | 22.0001 / 22.0001 | 2196.01 / 2196.01 | PASS |
| rp | mt | 35,141.4 | 35,146.6 | 5.1 | 28.0 (A vs A' in this matrix) | +0.2 x | 3 of 6 | 22.0002 / 22.0002 | 2196.02 / 2196.02 | PASS |
| rpm | ct | 92,877.0 | 92,877.5 | 0.6 | 475.1 (A vs A' in this matrix) | +0.0 x | 3 of 6 | 141.0006 / 141.0005 | 18797.06 / 18797.06 | PASS |
| rpm | mt | 95,389.5 | 95,255.5 | -134.0 | 682.7 (A vs A' in this matrix) | -0.2 x | 5 of 6 | 141.0013 / 141.0009 | 18797.14 / 18797.09 | PASS |
| wp | ct | 42,546.3 | 42,270.4 | -275.9 | 107.1 (A vs A' in this matrix) | -2.6 x | 6 of 6 | 27.0002 / 27.0002 | 8011.02 / 8011.02 | FAIL |
| wp | mt | 43,804.5 | 43,491.7 | -312.8 | 206.4 (A vs A' in this matrix) | -1.5 x | 6 of 6 | 27.0002 / 27.0002 | 8011.02 / 8011.02 | FAIL |

Baseline builds (A and A') byte-identical: True
Gates aimed at: wp. Accepted: False
