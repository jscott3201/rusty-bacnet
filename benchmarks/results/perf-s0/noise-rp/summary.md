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
| rp | ct | 40,809.7 | 40,809.7 | -0.1 | 15.5 | 2 of 6 | 22.0002 / 22.0002 | 2196.02 / 2196.02 | floor |
| rp | mt | 41,682.1 | 41,688.1 | 6.0 | 57.8 | 2 of 6 | 22.0003 / 22.0003 | 2196.03 / 2196.03 | floor |

Binaries identical: True
