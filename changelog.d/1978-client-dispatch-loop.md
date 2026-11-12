---
section: Changed
---
- The client's dispatch loop polls its idle queues only when it has nothing else to do, and the device purge runs as a task of its own: about 760 fewer instructions per ReadProperty round trip in a release-like build (#1978).
