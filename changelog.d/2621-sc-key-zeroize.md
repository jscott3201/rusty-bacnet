---
section: Security
---
- BACnet/SC private-key buffers (the key file text, decoded keys on every error path, the Python bytearray snapshot) are cleared on drop, and the TLS configs keep no key bytes (#2621). The PEM decoder and rustls copies cannot be cleared.
