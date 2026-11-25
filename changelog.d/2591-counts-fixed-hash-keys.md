---
section: Changed
---
- The counting harness fixes its hash-map keys in every run, so the ReadPropertyMultiple and WriteProperty noise floors no longer swing with the standard library's per-process seed; production builds keep their random keys (#2591).
