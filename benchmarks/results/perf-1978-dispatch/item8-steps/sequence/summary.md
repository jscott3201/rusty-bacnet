# Counting harness: compare run

- A: `d16ef2022bba3d77833698309dd7451fea63ae4f` (d16ef2022); binaries codec-counts `3cce4c5f6b91b611`, counts `5541269472d9ff82`
- A': `d16ef2022bba3d77833698309dd7451fea63ae4f` (d16ef2022); binaries codec-counts `3cce4c5f6b91b611`, counts `5541269472d9ff82`
- B: `ae89b50edfa96d0d0074e3340ec82ad1fd127542` (ae89b50ed); binaries codec-counts `3cce4c5f6b91b611`, counts `11065a587a1ea783`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 30,375.5 | 30,230.8 | -144.7 | 6.0 (A vs A' in this matrix) | -23.9 x | 6 of 6 | 18.0001 / 18.0001 | 2127.01 / 2127.02 | PASS |
| rp | mt | 31,666.3 | 31,519.4 | -147.0 | 27.2 (A vs A' in this matrix) | -5.4 x | 6 of 6 | 18.0003 / 18.0002 | 2127.03 / 2127.02 | PASS |
| rpm | ct | 90,382.0 | 90,237.1 | -144.9 | 45.9 (A vs A' in this matrix) | -3.2 x | 6 of 6 | 139.0004 / 139.0004 | 19000.04 / 19000.04 | PASS |
| rpm | mt | 92,519.1 | 92,124.2 | -394.9 | 826.9 (A vs A' in this matrix) | -0.5 x | 5 of 6 | 139.0004 / 139.0004 | 19000.04 / 19000.04 | PASS |
| wp | ct | 39,309.5 | 39,164.0 | -145.5 | 127.3 (A vs A' in this matrix) | -1.1 x | 6 of 6 | 25.0002 / 25.0002 | 8448.02 / 8448.02 | PASS |
| wp | mt | 40,510.4 | 40,344.2 | -166.2 | 181.0 (A vs A' in this matrix) | -0.9 x | 6 of 6 | 25.0003 / 25.0002 | 8448.04 / 8448.02 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: rp. Accepted: True
