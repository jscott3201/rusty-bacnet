---
section: Migration notes
---
- **Server recipients at a remote broadcast form (wire, #2732):** a confirmed
  event or audit recipient, Device binding or Audit Log parent whose MAC on
  another network is a broadcast or group form now gets nothing and counts in
  `confirmed_broadcast_recipient` (events). Name the station's own MAC, or use
  unconfirmed notifications; ARCNET station 255 takes only unconfirmed ones.
