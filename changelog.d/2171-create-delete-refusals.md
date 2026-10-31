---
section: Fixed
---
- **Wire:** CreateObject tells a type the device supports but doesn't create
  apart from an unsupported one, and refuses the reserved instance 4194303;
  DeleteObject reports an absent object as unknown and asks the object
  whether it may be deleted (#2171).
