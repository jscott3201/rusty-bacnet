# Counting harness: compare run

- A: `a13238fbb0237c8efb713e820cc2591e5d75d222` (a13238fbb); binaries codec-counts `7940fe59f9e28f3f`, counts `8ebd77453f69ce0a`
- A': `a13238fbb0237c8efb713e820cc2591e5d75d222` (a13238fbb); binaries codec-counts `7940fe59f9e28f3f`, counts `8ebd77453f69ce0a`
- B: `23906182f17529ab291a65db0aaaf490bfac4e32` (23906182f); binaries codec-counts `f423d4780be75e46`, counts `7c5e8f16f36b4fb1`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| decode_apdu_confirmed_request | none | 239.0 | 239.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 3 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| decode_npdu_100b_payload | none | 154.0 | 154.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 2 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| decode_read_property_request | none | 169.0 | 169.0 | -0.0 | 0.0 (A vs A' in this matrix) | 0 x | 3 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| encode_apdu_confirmed_request | none | 522.0 | 522.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 2 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | PASS |
| encode_npdu_100b_payload | none | 446.0 | 447.0 | 1.0 | 0.0 (A vs A' in this matrix) | +inf | 0 of 6 | 1.0000 / 1.0000 | 128.00 / 128.00 | FAIL |
| encode_read_property_request | none | 611.0 | 611.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 3 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | PASS |
| full_stack_encode_rp | none | 2,063.0 | 2,073.0 | 10.0 | 0.0 (A vs A' in this matrix) | +inf | 0 of 6 | 5.0000 / 5.0000 | 242.00 / 242.00 | FAIL |

Baseline builds (A and A') byte-identical: True
Gates aimed at: encode_npdu_100b_payload, full_stack_encode_rp. Accepted: False
