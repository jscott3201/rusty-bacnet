---
section: Fixed
---
- A server's Who-Is and Who-Has answers, and an endpoint's I-Am broadcast, no
  longer hold the object database lock while the reply is sent, so a slow link
  can't stall database writes and the reads queued behind them (#2199).
