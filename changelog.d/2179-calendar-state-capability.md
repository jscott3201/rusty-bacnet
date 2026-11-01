---
section: Changed
---
- A Calendar's state for a given day moves from the hidden
  `calendar_state_internal` hook to a hidden `CalendarState` capability lent
  through `calendar_state()`; its method is now `state_on` (#2179).
