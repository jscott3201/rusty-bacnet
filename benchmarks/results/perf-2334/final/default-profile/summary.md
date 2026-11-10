# Counting harness: compare run

- A: `2142f359ee095fcac17b79c6629f1eaddc434ccd` (2142f359e); binaries codec-counts `b3469d7299c697d4`, counts `5533740a0d91cf0d`
- A': `2142f359ee095fcac17b79c6629f1eaddc434ccd` (2142f359e); binaries codec-counts `b3469d7299c697d4`, counts `5533740a0d91cf0d`
- B: `78a2bffb2d8fadfdca7c2b9dade7c7a9e723a967` (78a2bffb2); binaries codec-counts `5362664c8377feb8`, counts `10673bf3af2eefac`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| decode_apdu_confirmed_request | none | 239.0 | 239.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 1 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| decode_npdu_100b_payload | none | 219.0 | 219.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 1 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| decode_read_property_request | none | 169.0 | 169.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 1 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| encode_apdu_confirmed_request | none | 522.0 | 522.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 1 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | PASS |
| encode_npdu_100b_payload | none | 446.0 | 446.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 0 of 6 | 1.0000 / 1.0000 | 128.00 / 128.00 | PASS |
| encode_read_property_request | none | 705.0 | 611.0 | -94.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | PASS |
| full_stack_encode_rp | none | 2,157.0 | 2,063.0 | -94.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 5.0000 / 5.0000 | 242.00 / 242.00 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: encode_read_property_request, full_stack_encode_rp. Accepted: True
