---
section: Fixed
---
- A priority read or a relinquish's read back whose request was never sent (no route, or a link refusing it) fails with that error, where it reported a property that did not decode; an object read fails once the same way (#2610).
