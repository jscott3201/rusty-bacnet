# Counting harness: compare run

- A: `32c21b87d9660a8a0bc18d9c3ab5af120661cdbf` (32c21b87d9660a8a0bc18d9c3ab5af120661cdbf); binaries codec-counts `c3ffb1fcae2eb82e`, counts `72117bef42c6611e`
- A': `32c21b87d9660a8a0bc18d9c3ab5af120661cdbf` (32c21b87d9660a8a0bc18d9c3ab5af120661cdbf); binaries codec-counts `c3ffb1fcae2eb82e`, counts `72117bef42c6611e`
- B: `9b9fa96b9bf672b3574be8a5b985d610fdd6a6d0` (9b9fa96b9bf672b3574be8a5b985d610fdd6a6d0); binaries codec-counts `4117a2b246a4832f`, counts `e3c55dbd2c77f605`

Instructions are callgrind instruction reads per request (per iteration for the codec
workloads), without the harness's counting allocator: the median over six ordinals of the
slope between the short and the long run. Calls are allocator calls per request (gated);
bytes are requested bytes per request (reported only).

| Workload | Runtime | A median | B median | B - A | Noise floor (source) | Change / floor | Pairs lower | Calls A / B | Bytes A / B (report only) | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rp | ct | 32,145.0 | 31,261.5 | -883.5 | 12.0 (A vs A' in this matrix) | -73.4 x | 6 of 6 | 22.0005 / 18.0005 | 2196.05 / 2127.05 | PASS |
| rp | mt | 33,539.8 | 32,632.7 | -907.2 | 46.6 (A vs A' in this matrix) | -19.5 x | 6 of 6 | 22.0008 / 18.0007 | 2196.08 / 2127.07 | PASS |
| rpm | ct | 91,289.5 | 91,015.2 | -274.2 | 873.0 (A vs A' in this matrix) | -0.3 x | 4 of 6 | 141.0011 / 139.0011 | 18781.11 / 18496.11 | PASS |
| rpm | mt | 94,540.0 | 94,040.8 | -499.2 | 350.6 (A vs A' in this matrix) | -1.4 x | 6 of 6 | 141.0014 / 139.0014 | 18781.15 / 18496.15 | PASS |
| wp | ct | 40,323.2 | 39,902.4 | -420.8 | 111.5 (A vs A' in this matrix) | -3.8 x | 6 of 6 | 27.0006 / 25.0006 | 7995.06 / 7944.06 | PASS |
| wp | mt | 41,581.6 | 41,128.8 | -452.8 | 146.3 (A vs A' in this matrix) | -3.1 x | 6 of 6 | 27.0009 / 25.0009 | 7995.09 / 7944.09 | PASS |

Baseline builds (A and A') byte-identical: True
Gates aimed at: rp. Accepted: True
