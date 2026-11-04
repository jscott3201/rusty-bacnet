# Counting harness: compare run

- A: `3b243714c47f3b2c3530bfab96265fc43e753c55` (scratch/perf-s5-item3-on-dev); binaries codec-counts `bbaf0ce4336da91f`, counts `b399fe34c5c92dc2`
- A': `3b243714c47f3b2c3530bfab96265fc43e753c55` (scratch/perf-s5-item3-on-dev); binaries codec-counts `bbaf0ce4336da91f`, counts `b399fe34c5c92dc2`
- B: `652f94c09dc4085c9c8be50ae5a7143ce286ed7d` (652f94c09); binaries codec-counts `4117a2b246a4832f`, counts `e3c55dbd2c77f605`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 31,668.7 | 31,250.1 | -418.6 | 9.4 (A vs A' in this matrix) | -44.3 x | 6 of 6 | 20.0001 / 18.0001 | 2183.02 / 2127.02 | PASS |
| rp | mt | 32,968.2 | 32,543.6 | -424.6 | 25.4 (A vs A' in this matrix) | -16.7 x | 6 of 6 | 20.0002 / 18.0002 | 2183.02 / 2127.02 | PASS |
| rpm | ct | 91,430.4 | 90,986.4 | -444.0 | 30.2 (A vs A' in this matrix) | -14.7 x | 6 of 6 | 141.0004 / 139.0004 | 18781.04 / 18496.04 | PASS |
| rpm | mt | 93,582.7 | 92,971.4 | -611.3 | 530.0 (A vs A' in this matrix) | -1.2 x | 5 of 6 | 141.0005 / 139.0005 | 18781.05 / 18496.05 | PASS |
| wp | ct | 40,309.4 | 39,887.9 | -421.5 | 101.5 (A vs A' in this matrix) | -4.2 x | 6 of 6 | 27.0002 / 25.0002 | 7995.02 / 7944.02 | PASS |
| wp | mt | 41,562.3 | 41,083.7 | -478.6 | 109.9 (A vs A' in this matrix) | -4.4 x | 6 of 6 | 27.0002 / 25.0003 | 7995.02 / 7944.03 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: rp. Accepted: True
