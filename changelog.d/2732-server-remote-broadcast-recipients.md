---
section: Changed
---
- **Breaking (wire):** the server starts no confirmed notification, write or
  read to a station on another network whose MAC is a broadcast or group form
  there (`0xFF`, `0xFFFFFF`, six octets starting `FF:FF:FF:FF`, LonTalk
  forms); it skips and counts it, and unconfirmed ones still go (#2732).
