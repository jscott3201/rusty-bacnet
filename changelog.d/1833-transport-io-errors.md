---
section: Changed
---
- **Breaking (Rust API):** I/O failures the transports reported as
  `Error::Encoding` text (serial open, read and write, GPIO, RS-485 ioctl,
  entropy, an SC direct bind) are `Error::Transport` with the OS
  error kept; an SC dial whose TLS session has no peer certificate is a TLS
  error (#1833).
