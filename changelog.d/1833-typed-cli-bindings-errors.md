---
section: Changed
---
- **Breaking (Rust API):** the CLI's option refusals, the Python bindings'
  own refusals and the test transport's injected failures use the typed
  variants; a test transport's scripted send failure is now
  `Error::Transport`, as a link's is, and CLI messages lose their leading
  `encoding error: ` (#1833).
