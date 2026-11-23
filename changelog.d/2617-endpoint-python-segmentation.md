---
section: Changed
---
- **Breaking (wire, Python API):** The endpoint classes' `segmentation` defaults to `Segmentation.SEGMENTED_BOTH`,
  which the endpoint's server and `client()` both follow, and no value is refused any more (#2617).
