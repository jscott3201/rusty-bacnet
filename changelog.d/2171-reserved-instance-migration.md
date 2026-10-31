---
section: Migration notes
---
- **Object database (Rust API, #2171):** `ObjectDatabase::add` refuses an
  object at instance 4194303, the reserved wildcard, a Device included. A
  Python `BACnetServer` holding one, or built with `device_instance=4194303`,
  now raises `ValueError` at `start()`. Give the object an instance from 0 to 4194302.
