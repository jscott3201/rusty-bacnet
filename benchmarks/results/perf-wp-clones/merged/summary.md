# Counting harness: compare run

- A: `03fb3bbbf4c685643f4a9f24e36fe44f718d2d10` (03fb3bbbf); binaries codec-counts `38564a1177fa4f5f`, counts `c90240273565b4f7`
- A': `03fb3bbbf4c685643f4a9f24e36fe44f718d2d10` (03fb3bbbf); binaries codec-counts `38564a1177fa4f5f`, counts `c90240273565b4f7`
- B: `a2ec0f8e09c1468df1a8039a2c4b5a7106e69602` (a2ec0f8e0); binaries codec-counts `72260765b96767cb`, counts `41017f290e43db23`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 33,241.8 | 33,245.1 | 3.3 | 10.3 (A vs A' in this matrix) | +0.3 x | 2 of 6 | 22.0001 / 22.0001 | 2196.01 / 2196.01 | PASS |
| rp | mt | 34,548.1 | 34,547.6 | -0.5 | 62.3 (A vs A' in this matrix) | -0.0 x | 4 of 6 | 22.0002 / 22.0002 | 2196.02 / 2196.02 | PASS |
| rpm | ct | 92,218.3 | 92,205.4 | -12.9 | 56.2 (A vs A' in this matrix) | -0.2 x | 3 of 6 | 141.0005 / 141.0005 | 18797.05 / 18797.05 | PASS |
| rpm | mt | 94,782.4 | 94,802.4 | 19.9 | 1,059.7 (A vs A' in this matrix) | +0.0 x | 3 of 6 | 141.0006 / 141.0012 | 18797.06 / 18797.13 | PASS |
| wp | ct | 41,952.3 | 41,450.5 | -501.8 | 7.5 (A vs A' in this matrix) | -66.8 x | 6 of 6 | 27.0002 / 27.0002 | 8011.02 / 8011.02 | PASS |
| wp | mt | 43,176.8 | 42,706.3 | -470.5 | 110.5 (A vs A' in this matrix) | -4.3 x | 6 of 6 | 27.0005 / 27.0002 | 8011.06 / 8011.02 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: wp. Accepted: True
