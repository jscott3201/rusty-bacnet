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
| rpm | ct | 115,365.5 | 115,506.3 | 140.8 | 978.4 | 3 of 6 | 141.0005 / 141.0005 | 18797.05 / 18797.05 | floor |
| rpm | mt | 117,960.6 | 117,983.0 | 22.3 | 664.9 | 3 of 6 | 141.0006 / 141.0006 | 18797.06 / 18797.06 | floor |
| wp | ct | 49,889.4 | 49,891.4 | 2.0 | 94.0 | 1 of 6 | 27.0002 / 27.0002 | 8011.02 / 8011.02 | floor |
| wp | mt | 50,645.3 | 50,617.3 | -28.0 | 144.5 | 4 of 6 | 27.0003 / 27.0003 | 8011.03 / 8011.03 | floor |

Binaries identical: True
