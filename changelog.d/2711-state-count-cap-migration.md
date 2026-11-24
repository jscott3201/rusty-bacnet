---
section: Migration notes
---
- **Multi-state state count (Rust and Python API, #2711).** Give the multi-state constructors, `set_number_of_states` and the Python `add_multistate_*` builders 1 to `MAX_NUMBER_OF_STATES` (1024) states; more is `Error::OutOfRange`, `BacnetError` in Python.
