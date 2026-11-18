---
section: Changed
---
- **Wire:** the COVNotificationMultiple decoder accepts a timestamp and times
  of change that disagree (the clause asks for both or neither): it decodes
  with the times it carried, a confirmed one is acknowledged, and it is
  published. It was rejected. The encoder stays strict (#2654).
