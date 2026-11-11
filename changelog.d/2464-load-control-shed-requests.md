---
section: Fixed
---
- **Wire:** Load Control takes the Start_Time write it refused, and a
  client's shed request runs through pending, compliant and non-compliant on
  the Device clock; a partly unspecified Start_Time is VALUE_OUT_OF_RANGE,
  and a request is not kept across a restart (#2464).
