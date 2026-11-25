---
section: Fixed
---
- **Wire:** A WritePropertyMultiple that writes a Schedule and one of its
  targets no longer sends a timestamped COV-multiple subscriber a row of the
  target with its old Status_Flags before the settled one (#2582).
