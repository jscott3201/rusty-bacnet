# Counting harness: noise run

- A: `41200dd6dc7e4cdfde82c498a0189368cebb95e9` (41200dd6d); binaries codec-counts `009a978b79f893c5`, counts `9193ee231307d28b`
- A': `41200dd6dc7e4cdfde82c498a0189368cebb95e9` (41200dd6d); binaries codec-counts `009a978b79f893c5`, counts `9193ee231307d28b`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | A' median | A' - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / A' | Bytes A / A' (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 30,996.6 | 30,992.8 | -3.8 | 13.5 (A vs A' in this matrix) | -0.3 x | 5 of 6 | 18.0001 / 18.0001 | 2127.01 / 2127.01 | floor |
| rp | mt | 32,282.5 | 32,286.8 | 4.4 | 96.1 (A vs A' in this matrix) | +0.0 x | 2 of 6 | 18.0002 / 18.0002 | 2127.02 / 2127.02 | floor |
| rpm | ct | 90,458.2 | 90,440.9 | -17.2 | 95.3 (A vs A' in this matrix) | -0.2 x | 4 of 6 | 139.0004 / 139.0004 | 19000.04 / 19000.04 | floor |
| rpm | mt | 93,156.4 | 93,028.5 | -127.9 | 848.8 (A vs A' in this matrix) | -0.2 x | 3 of 6 | 139.0005 / 139.0005 | 19000.05 / 19000.05 | floor |
| wp | ct | 39,934.0 | 39,927.9 | -6.1 | 215.7 (A vs A' in this matrix) | -0.0 x | 5 of 6 | 25.0002 / 25.0002 | 8448.02 / 8448.02 | floor |
| wp | mt | 41,140.9 | 41,097.2 | -43.7 | 140.2 (A vs A' in this matrix) | -0.3 x | 5 of 6 | 25.0002 / 25.0002 | 8448.02 / 8448.02 | floor |

Baseline builds (A and A') byte-identical: True
