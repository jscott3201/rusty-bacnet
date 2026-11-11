---
section: Fixed
---
- **Wire:** When a Load Control request ends, its level, Start_Time,
  Shed_Duration and Duty_Window return to their idle values; a Start_Time
  written behind the default level, or after its Shed_Duration has run, is
  acknowledged and then reads unspecified (#2464).
