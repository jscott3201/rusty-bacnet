---
section: Changed
---
- A dropped local call that has taken its locks may still make its change. A
  dropped local write lets go of its staged save when its task next runs, not
  in the drop, so `try_read` or `try_write` right after can find the lock still
  queued for (#2772).
