---
section: Fixed
---
- **Wire:** Load Control takes the Start_Time write it refused, and a
  client's shed request runs through pending, compliant and non-compliant on
  the Device clock; a Start_Time with part of its date or time left out is
  refused, and a request is not kept across a restart (#2464).
