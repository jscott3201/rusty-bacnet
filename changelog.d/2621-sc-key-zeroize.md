---
section: Security
---
- BACnet/SC key loading clears the key file's text and every decoded key on drop, error paths and FIFO reads included, as does the Python PEM inspection for its bytearray copies; the TLS configs keep no copy of the key's PEM or DER (#2621). The PEM decoder's own buffers are not cleared.
