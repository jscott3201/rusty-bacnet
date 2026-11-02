---
section: Added
---
- **Wire:** CreateObject now builds Calendar, Schedule, Notification Class,
  Trend Log, Event Enrollment and Structured View. Each starts empty, takes
  read-only settings such as Object_Name or a Trend Log's Buffer_Size (up to
  10,000) as initial values, and runs once configured, with no restart (#2208).
