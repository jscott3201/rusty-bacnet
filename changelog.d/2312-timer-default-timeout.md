---
section: Added
---
- A Timer built `with_persistence` keeps a Default_Timeout a peer writes
  across a restart; a saved value outside a range the application has since
  narrowed is dropped with a warning, and the rest still restores (#2312).
