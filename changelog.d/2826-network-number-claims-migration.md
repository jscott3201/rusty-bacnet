---
section: Migration notes
---
- **Network number (wire, #2826):** a standalone client takes a new number
  when every answer in its question's window (the APDU timeout or 10 s,
  whichever is longer, plus 2 s) names it and a router check, one APDU
  timeout, finds no router routing there: about 18 s at the defaults, so
  #2572's fold comes later. Endpoints and servers still take any
  announcement: configure their Network Port's number.
