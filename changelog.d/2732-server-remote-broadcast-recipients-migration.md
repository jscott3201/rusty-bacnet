---
section: Migration notes
---
- **Recipients at a remote broadcast form (wire, #2732):** a confirmed
  recipient, Device binding or Audit Log parent whose MAC on another network
  is a broadcast or group form gets nothing now; name the station's own MAC,
  or send unconfirmed. A device that is one station at such an address, such
  as ARCNET's 255, takes a routed Device binding marked one station (#2738).
