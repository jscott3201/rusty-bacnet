# Counting harness: noise run

- A: `64bb7eb27f0b27657f4591b2c9a5dfc06efaf097` (HEAD); binaries codec-counts `32775c9c26224381`, counts `c33f99118823c920`
- A': `64bb7eb27f0b27657f4591b2c9a5dfc06efaf097` (HEAD); binaries codec-counts `32775c9c26224381`, counts `c33f99118823c920`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | A' median | A' - A | Noise floor (source) | Pairs lower | Calls A / A' | Bytes A / A' (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| decode_apdu_confirmed_request | none | 239.0 | 239.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | floor |
| decode_npdu_100b_payload | none | 219.0 | 219.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | floor |
| decode_read_property_request | none | 169.0 | 169.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | floor |
| encode_apdu_confirmed_request | none | 522.0 | 522.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | floor |
| encode_npdu_100b_payload | none | 446.0 | 446.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 of 6 | 1.0000 / 1.0000 | 128.00 / 128.00 | floor |
| encode_read_property_request | none | 705.0 | 705.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | floor |
| full_stack_encode_rp | none | 2,157.0 | 2,157.0 | 0.0 | 0.0 (A vs A' in this matrix) | 0 of 6 | 5.0000 / 5.0000 | 242.00 / 242.00 | floor |

Baseline builds (A and A') byte-identical: True
