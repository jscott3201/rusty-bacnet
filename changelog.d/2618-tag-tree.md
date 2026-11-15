---
section: Changed
---
- **Breaking (Python API):** a read value with a context tag and no typed
  form (a vendor property, a typed read that fails to decode) is `tagged`,
  not one `application_data` blob: `.value` is the tree of its tags as
  `TagNode` records, `.octets` exactly as served (#2618).
