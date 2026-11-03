---
section: Fixed
---
- **Breaking (wire):** A SubscribeCOV asking for an indefinite lifetime from a
  server whose COV policy refuses them is answered SERVICES /
  VALUE_OUT_OF_RANGE, the code the standard lists, instead of
  OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED (#2317).
