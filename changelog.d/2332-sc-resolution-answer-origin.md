---
section: Fixed
---
- **Wire:** A BACnet/SC node accepts an Address-Resolution answer only from the node it asked, so another node on the hub can't forge one by guessing the message ID; a refusal from the hub itself ends the wait at once and caches nothing (#2332).
