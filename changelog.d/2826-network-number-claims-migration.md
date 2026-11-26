---
section: Migration notes
---
- **Network number (wire, #2826):** a standalone client takes its number
  after its question's window (the APDU timeout or 10 s, whichever is
  longer, plus 2 s) when every answer names it, stricter than Clause
  6.4.20 for mixed flags; a routed network waits one router check more, so
  #2572's fold comes later. Endpoints and servers still take any
  announcement: configure their Network Port's number.
