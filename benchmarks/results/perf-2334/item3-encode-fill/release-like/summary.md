# Counting harness: compare run

- A: `a13238fbb0237c8efb713e820cc2591e5d75d222` (a13238fbb); binaries codec-counts `d2ce0b8743eba813`, counts `67f5488b072e1cb3`
- A': `a13238fbb0237c8efb713e820cc2591e5d75d222` (a13238fbb); binaries codec-counts `d2ce0b8743eba813`, counts `67f5488b072e1cb3`
- B: `23906182f17529ab291a65db0aaaf490bfac4e32` (23906182f); binaries codec-counts `53b7b7d8496084c1`, counts `6dad81302412bc70`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| decode_apdu_confirmed_request | none | 222.0 | 222.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 0 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| decode_npdu_100b_payload | none | 153.0 | 153.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 1 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| decode_read_property_request | none | 180.0 | 180.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 2 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| encode_apdu_confirmed_request | none | 312.0 | 312.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 1 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | PASS |
| encode_npdu_100b_payload | none | 384.0 | 359.0 | -25.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 1.0000 / 1.0000 | 128.00 / 128.00 | PASS |
| encode_read_property_request | none | 306.0 | 306.0 | -0.0 | 0.0 (A vs A' in this matrix) | 0 x | 3 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | PASS |
| full_stack_encode_rp | none | 1,412.0 | 1,396.0 | -16.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 5.0000 / 5.0000 | 242.00 / 242.00 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: encode_npdu_100b_payload, full_stack_encode_rp. Accepted: True
