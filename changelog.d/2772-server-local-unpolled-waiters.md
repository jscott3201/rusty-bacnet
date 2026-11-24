---
section: Fixed
---
- A server's local reads and writes, `generate_pics`, `remove_peer_subscriptions`
  and an endpoint's `broadcast_i_am` no longer wait for a lock in their caller's
  task, so a call left unpolled holds up no request; one dropped before its
  turn changes nothing (#2772).
