---
section: Fixed
---
- A segmented answer that answers no request of an endpoint session now
  reaches the session's `client()`, which aborts it as an idle client does; the node
  sent nothing back before (#2617).
