---
section: Fixed
---
- Built-in objects called directly through `read_property` now refuse an
  array index on a property that isn't an array, as the server does; this
  includes a proprietary property the object lacks. Nothing changes on the
  wire (#2274).
