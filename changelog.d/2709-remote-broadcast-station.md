---
section: Changed
---
- **Breaking (wire):** a confirmed request to a station on another network
  whose MAC is a broadcast or group there (`0xFF`; two octets beginning
  `0x00` or `0xFF`; `0xFFFFFF`; six beginning `FF:FF:FF:FF`) is refused
  before anything is sent, and so are those bacnet-ops addresses (#2709).
