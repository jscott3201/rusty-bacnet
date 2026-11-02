# Counting harness: noise run

- A: `9764c9db8c264ab873a9db5b3a4dc14f643c38e3` (9764c9db8); binaries codec-counts `c87c02a3cd0826a3`, counts `b03c316267075659`
- A': `9764c9db8c264ab873a9db5b3a4dc14f643c38e3` (9764c9db8); binaries codec-counts `c87c02a3cd0826a3`, counts `b03c316267075659`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | A' median | A' - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / A' | Bytes A / A' (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| decode_apdu_confirmed_request | none | 222.0 | 222.0 | -0.0 | 0.0 (A vs A' in this matrix) | -0.3 x | 3 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | floor |
| decode_npdu_100b_payload | none | 172.0 | 172.0 | -0.0 | 0.0 (A vs A' in this matrix) | -0.3 x | 3 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | floor |
| decode_read_property_request | none | 180.0 | 180.0 | 0.0 | 0.0 (A vs A' in this matrix) | +0.0 x | 1 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | floor |
| encode_apdu_confirmed_request | none | 312.0 | 312.0 | 0.0 | 0.0 (A vs A' in this matrix) | +0.0 x | 2 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | floor |
| encode_npdu_100b_payload | none | 384.0 | 384.0 | 0.0 | 0.0 (A vs A' in this matrix) | +0.0 x | 3 of 6 | 1.0000 / 1.0000 | 128.00 / 128.00 | floor |
| encode_read_property_request | none | 306.0 | 306.0 | 0.0 | 0.0 (A vs A' in this matrix) | +0.0 x | 3 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | floor |
| full_stack_encode_rp | none | 1,412.0 | 1,412.0 | 0.0 | 0.0 (A vs A' in this matrix) | +0.2 x | 1 of 6 | 5.0000 / 5.0000 | 242.00 / 242.00 | floor |
| rp | ct | 33,829.5 | 33,832.3 | 2.8 | 10.6 (A vs A' in this matrix) | +0.3 x | 2 of 6 | 22.0002 / 22.0002 | 2196.02 / 2196.02 | floor |
| rp | mt | 35,213.2 | 35,213.0 | -0.1 | 130.8 (A vs A' in this matrix) | -0.0 x | 4 of 6 | 22.0003 / 22.0003 | 2196.04 / 2196.03 | floor |
| rpm | ct | 99,090.9 | 99,119.6 | 28.7 | 820.1 (A vs A' in this matrix) | +0.0 x | 2 of 6 | 141.0006 / 141.0005 | 18797.06 / 18797.06 | floor |
| rpm | mt | 102,119.2 | 102,064.8 | -54.4 | 751.2 (A vs A' in this matrix) | -0.1 x | 3 of 6 | 141.0007 / 141.0007 | 18797.07 / 18797.08 | floor |
| wp | ct | 42,688.4 | 42,677.9 | -10.5 | 107.7 (A vs A' in this matrix) | -0.1 x | 5 of 6 | 27.0003 / 27.0003 | 8011.03 / 8011.03 | floor |
| wp | mt | 43,964.4 | 43,888.0 | -76.4 | 155.0 (A vs A' in this matrix) | -0.5 x | 5 of 6 | 27.0004 / 27.0004 | 8011.04 / 8011.04 | floor |

Baseline builds (A and A') byte-identical: True
