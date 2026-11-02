---
section: Added
---
- **Wire:** CreateObject now builds Calendar, Schedule, Notification Class,
  Trend Log, Event Enrollment and Structured View. Each starts empty, takes
  its read-only configuration (such as a Trend Log's Buffer_Size) as initial
  values, and starts working once configured, without a restart (#2208).
