---
section: Security
---
- The client ignores a Reject-Message-To-Network that comes as a broadcast
  (a global one or one on its link), so one station can't end every
  client's requests to a network through a router at once; too-long
  rejects were open to this before too (#2724).
