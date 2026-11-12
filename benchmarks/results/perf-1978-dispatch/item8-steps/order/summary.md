# Counting harness: compare run

- A: `41200dd6dc7e4cdfde82c498a0189368cebb95e9` (41200dd6d); binaries codec-counts `009a978b79f893c5`, counts `9193ee231307d28b`
- A': `41200dd6dc7e4cdfde82c498a0189368cebb95e9` (41200dd6d); binaries codec-counts `009a978b79f893c5`, counts `9193ee231307d28b`
- B: `d16ef2022bba3d77833698309dd7451fea63ae4f` (d16ef2022); binaries codec-counts `3cce4c5f6b91b611`, counts `5541269472d9ff82`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 30,997.5 | 30,376.0 | -621.5 | 44.2 (A vs A' in this matrix) | -14.1 x | 6 of 6 | 18.0001 / 18.0001 | 2127.01 / 2127.01 | PASS |
| rp | mt | 32,279.7 | 31,666.2 | -613.6 | 36.6 (A vs A' in this matrix) | -16.8 x | 6 of 6 | 18.0002 / 18.0003 | 2127.02 / 2127.03 | PASS |
| rpm | ct | 90,446.7 | 90,382.6 | -64.0 | 50.1 (A vs A' in this matrix) | -1.3 x | 6 of 6 | 139.0004 / 139.0004 | 19000.04 / 19000.04 | PASS |
| rpm | mt | 92,961.0 | 92,629.6 | -331.4 | 500.8 (A vs A' in this matrix) | -0.7 x | 4 of 6 | 139.0005 / 139.0004 | 19000.05 / 19000.04 | PASS |
| wp | ct | 39,934.6 | 39,309.1 | -625.6 | 97.9 (A vs A' in this matrix) | -6.4 x | 6 of 6 | 25.0002 / 25.0002 | 8448.02 / 8448.02 | PASS |
| wp | mt | 41,129.1 | 40,482.9 | -646.2 | 99.9 (A vs A' in this matrix) | -6.5 x | 6 of 6 | 25.0003 / 25.0004 | 8448.04 / 8448.04 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: rp. Accepted: True
