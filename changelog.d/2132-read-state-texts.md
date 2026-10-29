---
section: Changed
---
- **Breaking (CLI output):** `bacnet read` and `readm` show a binary or
  multi-state state with the object's text, `2 (Low)`, for one extra read an
  object. JSON `value` changes for such states (`inactive` becomes
  `inactive (Inactive)`); scripts should use `decoded` (#2132).
