# Counting harness: noise run

- A: `64bb7eb27f0b27657f4591b2c9a5dfc06efaf097` (HEAD); binaries codec-counts `32775c9c26224381`, counts `c33f99118823c920`
- A': `64bb7eb27f0b27657f4591b2c9a5dfc06efaf097` (HEAD); binaries codec-counts `32775c9c26224381`, counts `c33f99118823c920`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | A' median | A' - A | Noise floor (source) | Pairs lower | Calls A / A' | Bytes A / A' (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rpm | ct | 109,397.5 | 109,538.3 | 140.8 | 978.4 (A vs A' in this matrix) | 3 of 6 | 141.0005 / 141.0005 | 18797.05 / 18797.05 | floor |
| rpm | mt | 111,988.8 | 112,010.8 | 22.0 | 664.7 (A vs A' in this matrix) | 3 of 6 | 141.0006 / 141.0006 | 18797.06 / 18797.06 | floor |
| wp | ct | 48,733.4 | 48,735.4 | 2.0 | 94.0 (A vs A' in this matrix) | 1 of 6 | 27.0002 / 27.0002 | 8011.02 / 8011.02 | floor |
| wp | mt | 49,486.9 | 49,458.9 | -28.0 | 146.9 (A vs A' in this matrix) | 4 of 6 | 27.0003 / 27.0003 | 8011.03 / 8011.03 | floor |

Baseline builds (A and A') byte-identical: True
