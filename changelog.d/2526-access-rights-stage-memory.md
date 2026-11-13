---
section: Fixed
---
- A WritePropertyMultiple with many rule writes to one Access Rights object
  no longer holds a copy of a rule array per write while its save runs, nor
  copies both arrays at each write it makes (#2526).
