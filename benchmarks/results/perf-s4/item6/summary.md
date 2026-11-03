# Counting harness: compare run

- A: `9b78cb492c40d58e920bd61294bc38e05d6b4f4c` (9b78cb492); binaries codec-counts `59d889d2f81c76ee`, counts `d7ce432fec88e7d7`
- A': `9b78cb492c40d58e920bd61294bc38e05d6b4f4c` (9b78cb492); binaries codec-counts `59d889d2f81c76ee`, counts `d7ce432fec88e7d7`
- B: `6a8bb8a85769ff7f8795ec7a8e490cfb9f1ea5a9` (6a8bb8a85); binaries codec-counts `59d889d2f81c76ee`, counts `9c7a67cab35f8b2c`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 32,406.1 | 32,130.5 | -275.6 | 37.6 (A vs A' in this matrix) | -7.3 x | 6 of 6 | 22.0002 / 22.0001 | 2196.02 / 2196.01 | PASS |
| rp | mt | 33,714.5 | 33,432.1 | -282.4 | 60.2 (A vs A' in this matrix) | -4.7 x | 6 of 6 | 22.0002 / 22.0002 | 2196.02 / 2196.02 | PASS |
| rpm | ct | 91,451.6 | 91,181.4 | -270.2 | 33.4 (A vs A' in this matrix) | -8.1 x | 6 of 6 | 141.0004 / 141.0004 | 18797.04 / 18797.04 | PASS |
| rpm | mt | 93,683.8 | 93,749.4 | 65.6 | 1,192.4 (A vs A' in this matrix) | +0.1 x | 4 of 6 | 141.0005 / 141.0005 | 18797.05 / 18797.05 | PASS |
| wp | ct | 40,624.7 | 40,348.8 | -275.9 | 13.3 (A vs A' in this matrix) | -20.7 x | 6 of 6 | 27.0002 / 27.0002 | 8011.02 / 8011.02 | PASS |
| wp | mt | 41,839.5 | 41,601.0 | -238.5 | 96.7 (A vs A' in this matrix) | -2.5 x | 6 of 6 | 27.0003 / 27.0003 | 8011.03 / 8011.03 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: rp. Accepted: True
