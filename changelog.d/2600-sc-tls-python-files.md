---
section: Changed
---
- **Python API:** SC credential files are read up to 1 MiB each, a larger one refused at startup, and `inspect_sc_pem` refuses a key input a connect refuses: two keys, an encrypted or OpenSSH key, or malformed PEM anywhere (#2600).
