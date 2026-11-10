# Counting harness: compare run

- A: `9e6bb098106239db36c2094916775ad8502b12ca` (9e6bb0981); binaries codec-counts `bf532b86bd141136`, counts `6c78eae2a69ea934`
- A': `9e6bb098106239db36c2094916775ad8502b12ca` (9e6bb0981); binaries codec-counts `bf532b86bd141136`, counts `6c78eae2a69ea934`
- B: `3b042d0e7ec9e087666042e17b8304475c463e6f` (3b042d0e7); binaries codec-counts `282dfceadac0f2bf`, counts `2a42f9f3328408db`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| decode_apdu_confirmed_request | none | 239.0 | 239.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 3 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| decode_npdu_100b_payload | none | 219.0 | 154.0 | -65.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| decode_read_property_request | none | 169.0 | 169.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 0 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| encode_apdu_confirmed_request | none | 522.0 | 522.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 1 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | PASS |
| encode_npdu_100b_payload | none | 446.0 | 446.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 3 of 6 | 1.0000 / 1.0000 | 128.00 / 128.00 | PASS |
| encode_read_property_request | none | 705.0 | 705.0 | -0.0 | 0.0 (A vs A' in this matrix) | 0 x | 2 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | PASS |
| full_stack_encode_rp | none | 2,157.0 | 2,157.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 2 of 6 | 5.0000 / 5.0000 | 242.00 / 242.00 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: decode_npdu_100b_payload. Accepted: True
