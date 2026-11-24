---
section: Changed
---
- **Breaking (CLI):** `subscribe` prints a notification's sender (`source_mac`
  in JSON, `source=` in text) in the link's form, as `discover` prints a
  peer: `192.168.1.10:47808` on BACnet/IP and `mstp:5` on MS/TP, not hex
  octets (#2783).
