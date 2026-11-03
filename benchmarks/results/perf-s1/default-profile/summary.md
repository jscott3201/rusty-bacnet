# Counting harness: compare run

- A: `dca94f7fa5acce74423606bf526de56d1f4360cf` (dca94f7f); binaries codec-counts `b68a3a221e05a224`, counts `f4c24c5271352d27`
- A': `dca94f7fa5acce74423606bf526de56d1f4360cf` (dca94f7f); binaries codec-counts `b68a3a221e05a224`, counts `f4c24c5271352d27`
- B: `a1d54afb8246e2b062d6bd5d408390c6d70ee02b` (a1d54afb8); binaries codec-counts `f1e56b374ace9128`, counts `a509f2d10f6a1739`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| decode_apdu_confirmed_request | none | 246.0 | 239.0 | -7.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| decode_npdu_100b_payload | none | 222.0 | 219.0 | -3.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| decode_read_property_request | none | 465.0 | 169.0 | -296.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| encode_apdu_confirmed_request | none | 532.0 | 522.0 | -10.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | PASS |
| encode_npdu_100b_payload | none | 408.0 | 446.0 | 38.0 | 0.0 (A vs A' in this matrix) | +inf | 0 of 6 | 1.0000 / 1.0000 | 128.00 / 128.00 | FAIL |
| encode_read_property_request | none | 503.0 | 705.0 | 202.0 | 0.0 (A vs A' in this matrix) | +inf | 0 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | FAIL |
| full_stack_encode_rp | none | 1,935.0 | 2,157.0 | 222.0 | 0.0 (A vs A' in this matrix) | +inf | 0 of 6 | 5.0000 / 5.0000 | 242.00 / 242.00 | FAIL |

Baseline builds (A and A') byte-identical: True
Gates aimed at: none. Accepted: False
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; rp/ct is missing
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; rp/mt is missing
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; wp/ct is missing
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; wp/mt is missing
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; rpm/ct is missing
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; rpm/mt is missing
