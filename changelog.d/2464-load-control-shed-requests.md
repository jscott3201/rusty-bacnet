---
section: Fixed
---
- **Wire:** Load Control serves Duty_Window, Enable, Shed_Levels and
  Shed_Level_Descriptions, marks Status_Flags required, and takes the
  Start_Time write it refused; a client's shed request now runs through
  pending, compliant and non-compliant on the Device clock (#2464).
