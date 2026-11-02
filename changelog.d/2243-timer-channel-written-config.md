---
section: Added
---
- Timers and Channels built `with_persistence` keep the configuration a peer
  writes across a restart: a Timer's State_Change_Values, references and
  Priority_For_Writing, and a Channel's members, delays, Channel_Number,
  Control_Groups and delay inhibit. A write that can't be saved is refused
  (#2243).
