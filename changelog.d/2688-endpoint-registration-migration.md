---
section: Migration notes
---
- **Python API (#2688).** Handle the error from endpoint `add_*` calls instead of from `start()`; earlier registrations stay in place for startup. Give Analog Input a finite value and use instances 0 through 4194302.
