# Counting harness: compare run

- A: `e9adf963089b10b5f646fcd6fc4f6f9f50f2f4e6` (e9adf9630); binaries codec-counts `46c6fcb06a125668`, counts `6a86ee6e75e4e8d1`
- A': `e9adf963089b10b5f646fcd6fc4f6f9f50f2f4e6` (e9adf9630); binaries codec-counts `46c6fcb06a125668`, counts `6a86ee6e75e4e8d1`
- B: `68f5f1411421c2165f4dbd0d6185d222777de4d3` (68f5f1411); binaries codec-counts `46c6fcb06a125668`, counts `7438aa3a2e452f0e`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 32,364.2 | 32,308.2 | -56.0 | 9.3 (A vs A' in this matrix) | -6.0 x | 6 of 6 | 18.0002 / 18.0002 | 2127.02 / 2119.02 | PASS |
| rp | mt | 33,682.7 | 33,616.9 | -65.8 | 158.5 (A vs A' in this matrix) | -0.4 x | 6 of 6 | 18.0005 / 18.0005 | 2127.05 / 2119.06 | PASS |
| rpm | ct | 91,501.1 | 91,195.8 | -305.4 | 50.6 (A vs A' in this matrix) | -6.0 x | 6 of 6 | 139.0004 / 138.0004 | 18512.04 / 17576.04 | PASS |
| rpm | mt | 94,247.1 | 93,963.8 | -283.4 | 824.5 (A vs A' in this matrix) | -0.3 x | 5 of 6 | 139.0004 / 138.0004 | 18512.04 / 17576.04 | PASS |
| wp | ct | 41,033.6 | 40,764.6 | -269.0 | 10.2 (A vs A' in this matrix) | -26.5 x | 6 of 6 | 25.0003 / 24.0003 | 7960.03 / 7024.03 | PASS |
| wp | mt | 42,285.0 | 41,977.4 | -307.6 | 187.4 (A vs A' in this matrix) | -1.6 x | 5 of 6 | 25.0010 / 24.0005 | 7960.10 / 7024.05 | FAIL |

Baseline builds (A and A') byte-identical: True
Gates aimed at: wp. Accepted: False
