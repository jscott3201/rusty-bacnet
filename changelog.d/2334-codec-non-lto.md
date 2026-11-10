---
section: Changed
---
- Builds without link-time optimization run fewer instructions encoding a ReadProperty request (94 fewer): the `BytesMut` sink's write is inlined. Builds with it are unchanged (#2334).
