---
section: Fixed
---
- A peer WriteProperty, WritePropertyMultiple or Add/RemoveListElement, the
  local write API (and the Command, Channel, Timer and WriteGroup writes made
  through it), Staging target writes and local access authorization commit the
  event-state transition with the write (#2530).
