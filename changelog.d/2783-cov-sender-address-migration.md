---
section: Migration notes
---
- **subscribe sender (CLI, #2783):** `source_mac` holds the sender's address
  as its link writes it, the form a target takes: `192.168.1.10:47808` on
  BACnet/IP, `bip6:0x0004D2` on BACnet/IPv6, `mstp:5` on MS/TP, and hex
  octets (`02:00:00:00:00:01`) only on BACnet/SC. A script that split it into
  hex octets reads it as that address instead.
