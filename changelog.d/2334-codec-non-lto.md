---
section: Changed
---
- Builds without link-time optimization run fewer instructions in two codecs: `decode_npdu` takes about 65 fewer per NPDU (19 fewer with it), and encoding a ReadProperty request about 94 fewer (#2334).
