---
section: Added
---
- **Wire:** Load Control serves Full_Duty_Baseline and State_Description,
  read-only and listed in Property_List, once the application sets them with
  `set_full_duty_baseline` and `set_state_description`; its `LoadShedder`
  reads the baseline from `ShedRequest` (#2479).
