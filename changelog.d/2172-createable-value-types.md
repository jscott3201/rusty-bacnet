---
section: Changed
---
- **Wire:** CreateObject now builds Analog Value and the twelve value types
  from one constructor table that the PICS also reads. Analog Value, Integer
  Value, Positive Integer Value and Large Analog Value take Units at
  creation, as Analog Input and Output already did (#2172).
