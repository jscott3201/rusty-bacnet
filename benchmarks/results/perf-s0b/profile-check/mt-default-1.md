# Counting harness: noise run

- A: `9764c9db8c264ab873a9db5b3a4dc14f643c38e3` (9764c9db8); binaries codec-counts `9a8fc8008b8854cc`, counts `5c8aa3587775e8d8`
- A': `9764c9db8c264ab873a9db5b3a4dc14f643c38e3` (9764c9db8); binaries codec-counts `9a8fc8008b8854cc`, counts `5c8aa3587775e8d8`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | A' median | A' - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / A' | Bytes A / A' (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | mt | 40,890.3 | 40,903.3 | 13.0 | 104.5 (A vs A' in this matrix) | +0.1 x | 4 of 6 | 22.0003 / 22.0003 | 2196.03 / 2196.03 | floor |
| wp | mt | 49,580.5 | 49,597.7 | 17.2 | 158.0 (A vs A' in this matrix) | +0.1 x | 1 of 6 | 27.0003 / 27.0003 | 8011.03 / 8011.03 | floor |

Baseline builds (A and A') byte-identical: True
