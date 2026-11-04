# Counting harness: compare run

- A: `40eeea7d9f87d1838e26f846d16823af40d4003a` (40eeea7d9); binaries codec-counts `59d889d2f81c76ee`, counts `b3214e5d6ccd2cc7`
- A': `40eeea7d9f87d1838e26f846d16823af40d4003a` (40eeea7d9); binaries codec-counts `59d889d2f81c76ee`, counts `b3214e5d6ccd2cc7`
- B: `9b78cb492c40d58e920bd61294bc38e05d6b4f4c` (9b78cb492); binaries codec-counts `59d889d2f81c76ee`, counts `d7ce432fec88e7d7`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 33,243.5 | 32,400.7 | -842.8 | 15.6 (A vs A' in this matrix) | -54.1 x | 6 of 6 | 22.0001 / 22.0001 | 2196.01 / 2196.01 | PASS |
| rp | mt | 34,556.8 | 33,709.2 | -847.6 | 80.9 (A vs A' in this matrix) | -10.5 x | 6 of 6 | 22.0002 / 22.0002 | 2196.02 / 2196.02 | PASS |
| rpm | ct | 92,119.3 | 91,458.6 | -660.6 | 227.0 (A vs A' in this matrix) | -2.9 x | 6 of 6 | 141.0005 / 141.0004 | 18797.05 / 18797.04 | PASS |
| rpm | mt | 94,708.0 | 94,213.2 | -494.8 | 762.5 (A vs A' in this matrix) | -0.6 x | 5 of 6 | 141.0007 / 141.0006 | 18797.08 / 18797.06 | PASS |
| wp | ct | 41,452.0 | 40,625.4 | -826.6 | 12.4 (A vs A' in this matrix) | -66.5 x | 6 of 6 | 27.0003 / 27.0003 | 8011.03 / 8011.03 | PASS |
| wp | mt | 42,775.0 | 41,915.0 | -859.9 | 234.3 (A vs A' in this matrix) | -3.7 x | 6 of 6 | 27.0007 / 27.0004 | 8011.07 / 8011.04 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: rp. Accepted: True
