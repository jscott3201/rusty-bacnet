---
section: Changed
---
- The client and server copy peer addresses fewer times per request, about 520 fewer instructions per ReadProperty round trip in a release-like build, and the server builds each reply route once (#1978).
