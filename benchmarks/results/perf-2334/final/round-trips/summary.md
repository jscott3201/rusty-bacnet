# Counting harness: compare run

- A: `2142f359ee095fcac17b79c6629f1eaddc434ccd` (2142f359e); binaries codec-counts `00a63321146442a8`, counts `9fb8e46b316b7ea2`
- A': `2142f359ee095fcac17b79c6629f1eaddc434ccd` (2142f359e); binaries codec-counts `00a63321146442a8`, counts `9fb8e46b316b7ea2`
- B: `78a2bffb2d8fadfdca7c2b9dade7c7a9e723a967` (78a2bffb2); binaries codec-counts `91328965c78038f0`, counts `8e7f9169f6c66664`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 31,011.9 | 31,012.7 | 0.7 | 17.6 (A vs A' in this matrix) | +0.0 x | 2 of 6 | 18.0001 / 18.0001 | 2127.01 / 2127.01 | PASS |
| rp | mt | 32,300.9 | 32,301.5 | 0.6 | 66.0 (A vs A' in this matrix) | +0.0 x | 4 of 6 | 18.0002 / 18.0002 | 2127.02 / 2127.02 | PASS |
| rpm | ct | 90,423.7 | 90,409.4 | -14.3 | 76.4 (A vs A' in this matrix) | -0.2 x | 4 of 6 | 139.0004 / 139.0004 | 19000.04 / 19000.04 | PASS |
| rpm | mt | 92,843.6 | 92,852.2 | 8.6 | 606.5 (A vs A' in this matrix) | +0.0 x | 2 of 6 | 139.0004 / 139.0005 | 19000.04 / 19000.05 | PASS |
| wp | ct | 39,919.7 | 39,915.5 | -4.2 | 12.1 (A vs A' in this matrix) | -0.3 x | 5 of 6 | 25.0002 / 25.0002 | 8448.02 / 8448.02 | PASS |
| wp | mt | 41,135.2 | 41,155.4 | 20.2 | 156.9 (A vs A' in this matrix) | +0.1 x | 3 of 6 | 25.0002 / 25.0002 | 8448.02 / 8448.02 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: none. Accepted: True
