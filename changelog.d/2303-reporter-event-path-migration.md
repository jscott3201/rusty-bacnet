---
section: Migration notes
---
- **Reporter event path (Rust API, #2303):** `Reporter::changed` and
  `Reporter::on_deadline` take `&mut ObjectDatabase` (a COV-only host may
  call `on_cov_deadline` with a shared one), and `event::distribute_event`
  takes a `RecipientWindow`: `RecipientWindow::Frame { fallback: Some(..) }`
  keeps the old fallback.
