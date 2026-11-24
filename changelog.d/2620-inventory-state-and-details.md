---
section: Added
---
- An inventory with values also reads each object's Event_State and Reliability, by name; the new `details` option (`bacnet inventory --details`) reads its Device_Type and Profile_Name. An object without the optional ones fails nothing (#2620).
