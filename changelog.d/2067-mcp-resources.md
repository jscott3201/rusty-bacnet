---
section: Added
---
- `bacnet mcp serve` offers paged resources for devices, points, alarms and
  the session, with subscriptions (on older protocol revisions too) sharing
  one finite COV subscription per point or polling, up to 4 listen streams,
  and a `bacnet_watch` tool for hosts without them (#2067, #2114, #2116,
  #2150).
