---
section: Added
---
- **CLI:** `subscribe --json` and a live `capture --json` mark a gap in
  their stream with a `missed` line counting the notifications or packets
  lost while the command fell behind; stderr notes it in every format
  (#2787).
