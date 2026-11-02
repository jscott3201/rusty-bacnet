# Counting harness: noise run

- A: `9764c9db8c264ab873a9db5b3a4dc14f643c38e3` (9764c9db8); binaries codec-counts `c87c02a3cd0826a3`, counts `b03c316267075659`
- A': `9764c9db8c264ab873a9db5b3a4dc14f643c38e3` (9764c9db8); binaries codec-counts `c87c02a3cd0826a3`, counts `b03c316267075659`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | A' median | A' - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / A' | Bytes A / A' (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| decode_apdu_confirmed_request | none | 222.0 | 222.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 1 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | floor |
| decode_npdu_100b_payload | none | 172.0 | 172.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 1 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | floor |
| decode_read_property_request | none | 180.0 | 180.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 1 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | floor |
| encode_apdu_confirmed_request | none | 312.0 | 312.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 0 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | floor |
| encode_npdu_100b_payload | none | 384.0 | 384.0 | -0.0 | 0.0 (A vs A' in this matrix) | 0 x | 3 of 6 | 1.0000 / 1.0000 | 128.00 / 128.00 | floor |
| encode_read_property_request | none | 306.0 | 306.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 1 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | floor |
| full_stack_encode_rp | none | 1,412.0 | 1,412.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 2 of 6 | 5.0000 / 5.0000 | 242.00 / 242.00 | floor |
| rp | ct | 33,827.9 | 33,825.5 | -2.4 | 11.4 (A vs A' in this matrix) | -0.2 x | 4 of 6 | 22.0001 / 22.0001 | 2196.01 / 2196.01 | floor |
| rp | mt | 35,134.8 | 35,132.7 | -2.0 | 14.8 (A vs A' in this matrix) | -0.1 x | 4 of 6 | 22.0002 / 22.0002 | 2196.02 / 2196.02 | floor |
| rpm | ct | 92,864.1 | 92,861.7 | -2.5 | 41.4 (A vs A' in this matrix) | -0.1 x | 3 of 6 | 141.0004 / 141.0004 | 18797.04 / 18797.04 | floor |
| rpm | mt | 95,234.1 | 95,134.7 | -99.3 | 641.0 (A vs A' in this matrix) | -0.2 x | 3 of 6 | 141.0004 / 141.0005 | 18797.04 / 18797.05 | floor |
| wp | ct | 42,651.2 | 42,649.5 | -1.7 | 9.9 (A vs A' in this matrix) | -0.2 x | 3 of 6 | 27.0002 / 27.0002 | 8011.02 / 8011.02 | floor |
| wp | mt | 43,868.0 | 43,886.7 | 18.6 | 146.0 (A vs A' in this matrix) | +0.1 x | 3 of 6 | 27.0002 / 27.0002 | 8011.02 / 8011.02 | floor |

Baseline builds (A and A') byte-identical: True
