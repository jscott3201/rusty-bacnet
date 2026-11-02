# Counting harness: compare run

- A: `a1d54afb8246e2b062d6bd5d408390c6d70ee02b` (a1d54afb8); binaries codec-counts `c87c02a3cd0826a3`, counts `a8282d44a0b6574d`
- A': `a1d54afb8246e2b062d6bd5d408390c6d70ee02b` (a1d54afb8); binaries codec-counts `c87c02a3cd0826a3`, counts `a8282d44a0b6574d`
- B: `c3086c3d7125ef20a90793e330db6109fcce6f0d` (c3086c3d7125ef20a90793e330db6109fcce6f0d); binaries codec-counts `fdc0a1c2fbc50d8c`, counts `a8282d44a0b6574d`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| decode_npdu_100b_payload | none | 172.0 | 154.0 | -18.0 | 0.0 (A vs A' in this matrix) | -inf | 6 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: none. Accepted: False
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; rp/ct is missing
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; rp/mt is missing
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; wp/ct is missing
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; wp/mt is missing
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; rpm/ct is missing
- Not acceptable as run: round trips need rp, wp and rpm on ct and mt; rpm/mt is missing
