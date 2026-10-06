---
section: Added
---
- **Rust API:** `BACnetObject::set_present_value_from_internal` hands an
  application Present_Value update the local Device it comes from, which
  `set_present_value_local` now passes; object decorators should forward it
  (#1552).
