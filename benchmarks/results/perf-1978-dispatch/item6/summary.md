# Counting harness: compare run

- A: `40044f639330032610ef24bd0309a81caf489c0a` (40044f639); binaries codec-counts `3cce4c5f6b91b611`, counts `7863a61b63eac15a`
- A': `40044f639330032610ef24bd0309a81caf489c0a` (40044f639); binaries codec-counts `3cce4c5f6b91b611`, counts `7863a61b63eac15a`
- B: `e721988a1e451bf0558977bad10b3f5d69d59a3b` (e721988a1); binaries codec-counts `3cce4c5f6b91b611`, counts `84463759444d17f3`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 30,230.4 | 30,231.6 | 1.2 | 9.4 (A vs A' in this matrix) | +0.1 x | 0 of 6 | 18.0001 / 18.0001 | 2127.01 / 2127.01 | PASS |
| rp | mt | 31,511.6 | 31,511.9 | 0.3 | 13.7 (A vs A' in this matrix) | +0.0 x | 3 of 6 | 18.0002 / 18.0002 | 2127.02 / 2127.02 | PASS |
| rpm | ct | 90,235.4 | 90,234.8 | -0.6 | 35.9 (A vs A' in this matrix) | -0.0 x | 3 of 6 | 139.0004 / 139.0004 | 19000.04 / 19000.04 | PASS |
| rpm | mt | 92,133.2 | 92,086.4 | -46.8 | 715.8 (A vs A' in this matrix) | -0.1 x | 5 of 6 | 139.0004 / 139.0004 | 19000.04 / 19000.04 | PASS |
| wp | ct | 39,165.4 | 39,008.6 | -156.9 | 92.1 (A vs A' in this matrix) | -1.7 x | 6 of 6 | 25.0002 / 25.0002 | 8448.02 / 8448.02 | FAIL |
| wp | mt | 40,359.3 | 40,202.4 | -156.9 | 60.8 (A vs A' in this matrix) | -2.6 x | 6 of 6 | 25.0002 / 25.0002 | 8448.02 / 8448.02 | FAIL |

Baseline builds (A and A') byte-identical: True
Gates aimed at: wp. Accepted: False
