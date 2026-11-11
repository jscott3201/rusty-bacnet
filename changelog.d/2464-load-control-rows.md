---
section: Fixed
---
- **Wire:** Load Control serves Duty_Window, Enable, Shed_Levels and
  Shed_Level_Descriptions, so its Property_List grows by four and its
  SubscribeCOV report gains Duty_Window; Shed_Levels refuses a level of 0,
  and Status_Flags is required (#2464).
