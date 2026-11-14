---
section: Added
---
- Binary Input and Binary Output apply a configurable Polarity between
  Present_Value and a named physical side: `set_physical_input_local` takes
  an input's raw state and `physical_output_local` gives an output's drive
  state (#2580). **Python API:** the same, and `polarity=` on both builders.
