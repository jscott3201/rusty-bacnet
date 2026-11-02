# Counting harness: noise run

- A: `64bb7eb27f0b27657f4591b2c9a5dfc06efaf097` (HEAD)
- B: `64bb7eb27f0b27657f4591b2c9a5dfc06efaf097` (HEAD)
- A binaries (SHA-256): codec-counts `32775c9c26224381`, counts `c33f99118823c920`
- B binaries (SHA-256): codec-counts `32775c9c26224381`, counts `c33f99118823c920`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads): the median over six A B B A ordinals of the slope between the short and the
long run. Calls and bytes are allocator calls and requested bytes per request.

| Workload | Runtime | A median | B median | B - A | Noise floor | Pairs lower | Calls A / B | Bytes A / B | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| decode_apdu_confirmed_request | none | 239.0 | 239.0 | 0.0 | 0.0 | 0 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | floor |
| decode_npdu_100b_payload | none | 219.0 | 219.0 | 0.0 | 0.0 | 0 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | floor |
| decode_read_property_request | none | 169.0 | 169.0 | 0.0 | 0.0 | 0 of 6 | 0.0000 / 0.0000 | 0.00 / 0.00 | floor |
| encode_apdu_confirmed_request | none | 564.0 | 564.0 | 0.0 | 0.0 | 0 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | floor |
| encode_npdu_100b_payload | none | 488.0 | 488.0 | 0.0 | 0.0 | 0 of 6 | 1.0000 / 1.0000 | 128.00 / 128.00 | floor |
| encode_read_property_request | none | 747.0 | 747.0 | 0.0 | 0.0 | 0 of 6 | 1.0000 / 1.0000 | 64.00 / 64.00 | floor |
| full_stack_encode_rp | none | 2,367.0 | 2,367.0 | 0.0 | 0.0 | 0 of 6 | 5.0000 / 5.0000 | 242.00 / 242.00 | floor |

Binaries identical: True
