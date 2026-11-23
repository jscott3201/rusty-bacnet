---
section: Changed
---
- **Python API:** The endpoint classes' `segmentation` defaults to `Segmentation.SEGMENTED_BOTH`,
  which the endpoint's server and `client()` both follow, and no value is refused any more;
  `BACnetServer()` keeps its `NO_SEGMENTATION` default (#2617).
