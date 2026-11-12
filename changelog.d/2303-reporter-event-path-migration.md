---
section: Migration notes
---
- `Reporter::changed` and `Reporter::on_deadline` take `&mut ObjectDatabase`; `event::distribute_event` takes a `RecipientWindow` (`RecipientWindow::Frame { fallback: Some(..) }` for the old fallback). (#2303).
