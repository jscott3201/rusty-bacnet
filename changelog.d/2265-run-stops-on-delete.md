---
section: Fixed
---
- **Wire:** A Command, Channel or Timer deleted while its writes are in
  progress now makes no further write, local or in another device, including
  retries and writes waiting on a device lookup (#2265).
