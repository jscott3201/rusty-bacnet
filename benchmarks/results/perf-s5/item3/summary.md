# Counting harness: compare run

- A: `32c21b87d9660a8a0bc18d9c3ab5af120661cdbf` (32c21b87d); binaries codec-counts `c3ffb1fcae2eb82e`, counts `72117bef42c6611e`
- A': `32c21b87d9660a8a0bc18d9c3ab5af120661cdbf` (32c21b87d); binaries codec-counts `c3ffb1fcae2eb82e`, counts `72117bef42c6611e`
- B: `3b243714c47f3b2c3530bfab96265fc43e753c55` (scratch/perf-s5-item3-on-dev); binaries codec-counts `bbaf0ce4336da91f`, counts `b399fe34c5c92dc2`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 32,134.1 | 31,672.4 | -461.7 | 13.5 (A vs A' in this matrix) | -34.2 x | 6 of 6 | 22.0002 / 20.0002 | 2196.02 / 2183.02 | PASS |
| rp | mt | 33,442.7 | 32,967.0 | -475.7 | 103.6 (A vs A' in this matrix) | -4.6 x | 6 of 6 | 22.0002 / 20.0002 | 2196.02 / 2183.02 | PASS |
| rpm | ct | 91,431.5 | 91,431.1 | -0.5 | 20.6 (A vs A' in this matrix) | -0.0 x | 3 of 6 | 141.0005 / 141.0005 | 18781.05 / 18781.05 | PASS |
| rpm | mt | 93,500.9 | 93,355.2 | -145.7 | 918.0 (A vs A' in this matrix) | -0.2 x | 4 of 6 | 141.0005 / 141.0006 | 18781.05 / 18781.06 | PASS |
| wp | ct | 40,305.3 | 40,306.8 | 1.5 | 10.0 (A vs A' in this matrix) | +0.2 x | 3 of 6 | 27.0002 / 27.0002 | 7995.02 / 7995.02 | PASS |
| wp | mt | 41,554.3 | 41,566.9 | 12.6 | 67.9 (A vs A' in this matrix) | +0.2 x | 3 of 6 | 27.0002 / 27.0002 | 7995.02 / 7995.02 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: rp. Accepted: True
