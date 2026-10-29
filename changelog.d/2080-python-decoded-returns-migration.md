---
section: Migration notes
---
- **Decoded returns (#2080):** `create_object` returns an object identifier,
  and the CLI prints `Created TYPE:instance`. Python
  `atomic_read_file`/`atomic_write_file` return dicts. Bit strings,
  `result_flags` and a log `bitstring` datum included, are `BitString`:
  `bits[i]` and iteration give bools, so read `.unused_bits` and `.data` where
  you unpacked a tuple, and compare `.bits`, not a tuple.
