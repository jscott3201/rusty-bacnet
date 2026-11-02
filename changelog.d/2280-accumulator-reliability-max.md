---
section: Fixed
---
- **Wire:** Accumulator Reliability is writable out of service and restored
  on return; Max_Pres_Value can't drop below Present_Value. **Rust API:**
  `set_present_value` stores modulo Max_Pres_Value + 1, and the Accumulator
  lends `ReliabilityEval` (`set_reliability`) (#2280).
