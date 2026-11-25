---
section: Fixed
---
- The client forgets a device's COV refusal when the discovery table loses
  its row or another device announces itself at its address, as it forgets
  the instance read there, instead of keeping it for the client's life
  (#2603).
