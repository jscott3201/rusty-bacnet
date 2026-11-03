---
section: Changed
---
- A durable log whose stored snapshot holds an invalid Start_Time, Stop_Time or reference now fails to open as damaged data (an encoding error; `BacnetError` in Python) rather than VALUE_OUT_OF_RANGE (`BacnetProtocolError`) (#2047).
