---
section: Migration notes
---
- **DCC and ReinitializeDevice passwords (#2060, #2094):** keep a configured
  `dcc_password` or `reinit_password` to 20 characters, and pass `None`
  instead of an empty string; either now fails at start (in Python, at
  construction) instead of refusing every request. The request encoders,
  Python client calls and the CLI's `--password ""` refuse an empty password
  too.
