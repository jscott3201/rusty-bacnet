# Counting harness: noise run

- A: `64bb7eb27f0b27657f4591b2c9a5dfc06efaf097` (HEAD); binaries codec-counts `32775c9c26224381`, counts `c33f99118823c920`
- A': `64bb7eb27f0b27657f4591b2c9a5dfc06efaf097` (HEAD); binaries codec-counts `32775c9c26224381`, counts `c33f99118823c920`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | A' median | A' - A | Noise floor (source) | Pairs lower | Calls A / A' | Bytes A / A' (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 39,864.7 | 39,864.6 | -0.1 | 15.5 (A vs A' in this matrix) | 2 of 6 | 22.0002 / 22.0002 | 2196.02 / 2196.02 | floor |
| rp | mt | 40,737.1 | 40,743.1 | 6.0 | 57.8 (A vs A' in this matrix) | 2 of 6 | 22.0003 / 22.0003 | 2196.03 / 2196.03 | floor |

Baseline builds (A and A') byte-identical: True
