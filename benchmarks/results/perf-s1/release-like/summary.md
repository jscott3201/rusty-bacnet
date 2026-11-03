# Counting harness: compare run

- A: `dca94f7fa5acce74423606bf526de56d1f4360cf` (dca94f7f); binaries codec-counts `68805077172540d3`, counts `84653d34d504f6fd`
- A': `dca94f7fa5acce74423606bf526de56d1f4360cf` (dca94f7f); binaries codec-counts `68805077172540d3`, counts `84653d34d504f6fd`
- B: `a1d54afb8246e2b062d6bd5d408390c6d70ee02b` (a1d54afb8); binaries codec-counts `c87c02a3cd0826a3`, counts `a8282d44a0b6574d`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| decode_apdu_confirmed_request | none | 241.0 | 222.0 | -19.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| decode_npdu_100b_payload | none | 219.0 | 172.0 | -47.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| decode_read_property_request | none | 344.0 | 180.0 | -164.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |
| encode_apdu_confirmed_request | none | 551.0 | 312.0 | -239.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | PASS |
| encode_npdu_100b_payload | none | 414.0 | 384.0 | -30.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 1.0000 / 1.0000 | 128.00 / 128.00 | PASS |
| encode_read_property_request | none | 453.0 | 306.0 | -147.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | PASS |
| full_stack_encode_rp | none | 1,838.0 | 1,412.0 | -426.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 5.0000 / 5.0000 | 242.00 / 242.00 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: none. Accepted: False
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; rp/ct is missing
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; rp/mt is missing
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; wp/ct is missing
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; wp/mt is missing
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; rpm/ct is missing
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; rpm/mt is missing
