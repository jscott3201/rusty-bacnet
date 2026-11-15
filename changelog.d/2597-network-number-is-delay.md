---
section: Changed
---
- **Wire:** a client, server or endpoint answers a broadcast What-Is-Network-Number
  after a random wait of 0.5 to 10 seconds, and not at all when another
  node's matching Network-Number-Is is heard first; a question sent to the
  node alone is still answered at once (#2597).
