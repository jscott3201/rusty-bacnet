---
section: Added
---
- **Python API:** `BacnetLookupError.answer` and `answer_instance` say what the
  address asked did (silent, another device, no such Device object, a
  refusal), and MCP refusals list the claimed addresses as `claims`, each
  marked unconfirmed (#2824).
