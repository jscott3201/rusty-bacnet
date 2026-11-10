---
section: Added
---
- **Breaking (Rust API):** Device and the two lighting outputs keep a
  peer-written Description in their Tags storage, and the Color objects in
  their output file; an endpoint's Device does too.
  `clear_written_description` drops a saved one (#2364). **Python API:**
  the colour builders' `storage_path`.
