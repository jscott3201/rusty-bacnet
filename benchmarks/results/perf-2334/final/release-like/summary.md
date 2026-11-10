# Counting harness: compare run

- A: `2142f359ee095fcac17b79c6629f1eaddc434ccd` (2142f359e); binaries codec-counts `00a63321146442a8`, counts `9fb8e46b316b7ea2`
- A': `2142f359ee095fcac17b79c6629f1eaddc434ccd` (2142f359e); binaries codec-counts `00a63321146442a8`, counts `9fb8e46b316b7ea2`
- B: `78a2bffb2d8fadfdca7c2b9dade7c7a9e723a967` (78a2bffb2); binaries codec-counts `91328965c78038f0`, counts `8e7f9169f6c66664`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| decode_apdu_confirmed_request | none | 222.0 | 222.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 1 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| decode_npdu_100b_payload | none | 172.0 | 172.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 1 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| decode_read_property_request | none | 180.0 | 180.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 0 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| encode_apdu_confirmed_request | none | 312.0 | 312.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 2 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | PASS |
| encode_npdu_100b_payload | none | 384.0 | 384.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 0 of 6 | 1.0000 / 1.0000 | 128.00 / 128.00 | PASS |
| encode_read_property_request | none | 306.0 | 306.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 0 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | FAIL |
| full_stack_encode_rp | none | 1,412.0 | 1,412.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 x | 1 of 6 | 5.0000 / 5.0000 | 242.00 / 242.00 | FAIL |

Baseline builds (A and A') byte-identical: True
Gates aimed at: encode_read_property_request, full_stack_encode_rp. Accepted: False
