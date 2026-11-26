---
section: Migration notes
---
- **Server and endpoint routers (wire, #2831):** a remote network's router is
  learned only from the node's own Who-Is-Router-To-Network, when exactly one
  router answers; until then requests go by local broadcast. A replaced router
  is taken about 1.5 minutes after the first unanswered request, retries going
  by broadcast meanwhile. Where answers return through another router than the
  requests take, the requests time out; watch `off_path_answer_drops`.
