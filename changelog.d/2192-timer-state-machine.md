---
section: Fixed
---
- **Breaking (wire):** The Timer object runs the Clause 12.57 state machine:
  Present_Value is the time left in milliseconds, writes to Present_Value,
  Timer_Running and Timer_State start, expire or clear it, and it serves
  Default_Timeout, Last_State_Change and its timeout range (#2192).
