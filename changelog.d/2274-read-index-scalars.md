---
section: Fixed
---
- Every built-in object now refuses an array index on a scalar property when
  `read_property` is called directly, as the server already did, through the
  new shared `check_read_array_index`. Nothing changes on the wire (#2274).
