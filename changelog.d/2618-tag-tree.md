---
section: Changed
---
- **Breaking (Python API):** a read value with a context tag and no typed
  form, a vendor property's included, is `tagged`, not one `application_data`
  blob: `.value` is the tree of its tags as `TagNode` records, and `.octets`
  stay exactly as served (#2618).
