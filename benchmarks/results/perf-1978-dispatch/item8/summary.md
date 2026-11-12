# Counting harness: compare run

- A: `41200dd6dc7e4cdfde82c498a0189368cebb95e9` (41200dd6d); binaries codec-counts `009a978b79f893c5`, counts `9193ee231307d28b`
- A': `41200dd6dc7e4cdfde82c498a0189368cebb95e9` (41200dd6d); binaries codec-counts `009a978b79f893c5`, counts `9193ee231307d28b`
- B: `40044f639330032610ef24bd0309a81caf489c0a` (40044f639); binaries codec-counts `3cce4c5f6b91b611`, counts `7863a61b63eac15a`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 30,998.6 | 30,229.9 | -768.7 | 9.0 (A vs A' in this matrix) | -85.5 x | 6 of 6 | 18.0001 / 18.0001 | 2127.01 / 2127.01 | PASS |
| rp | mt | 32,277.3 | 31,510.4 | -766.9 | 14.3 (A vs A' in this matrix) | -53.5 x | 6 of 6 | 18.0002 / 18.0002 | 2127.02 / 2127.02 | PASS |
| rpm | ct | 90,443.9 | 90,234.7 | -209.3 | 124.2 (A vs A' in this matrix) | -1.7 x | 6 of 6 | 139.0004 / 139.0004 | 19000.04 / 19000.04 | PASS |
| rpm | mt | 93,069.9 | 92,167.3 | -902.6 | 484.0 (A vs A' in this matrix) | -1.9 x | 6 of 6 | 139.0004 / 139.0004 | 19000.04 / 19000.04 | PASS |
| wp | ct | 39,928.5 | 39,167.4 | -761.1 | 97.7 (A vs A' in this matrix) | -7.8 x | 6 of 6 | 25.0002 / 25.0002 | 8448.02 / 8448.02 | PASS |
| wp | mt | 41,108.4 | 40,339.3 | -769.1 | 53.5 (A vs A' in this matrix) | -14.4 x | 6 of 6 | 25.0002 / 25.0002 | 8448.02 / 8448.02 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: rp. Accepted: True
