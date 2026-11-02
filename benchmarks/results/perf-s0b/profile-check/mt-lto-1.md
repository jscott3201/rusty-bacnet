# Counting harness: noise run

- A: `9764c9db8c264ab873a9db5b3a4dc14f643c38e3` (9764c9db8); binaries codec-counts `c87c02a3cd0826a3`, counts `b03c316267075659`
- A': `9764c9db8c264ab873a9db5b3a4dc14f643c38e3` (9764c9db8); binaries codec-counts `c87c02a3cd0826a3`, counts `b03c316267075659`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | A' median | A' - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / A' | Bytes A / A' (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | mt | 35,176.0 | 35,165.5 | -10.6 | 88.8 (A vs A' in this matrix) | -0.1 x | 4 of 6 | 22.0002 / 22.0002 | 2196.02 / 2196.02 | floor |
| wp | mt | 43,910.9 | 43,870.9 | -40.0 | 172.8 (A vs A' in this matrix) | -0.2 x | 5 of 6 | 27.0002 / 27.0002 | 8011.02 / 8011.02 | floor |

Baseline builds (A and A') byte-identical: True
