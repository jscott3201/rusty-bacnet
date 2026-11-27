---
section: Migration notes
---
- **Server and endpoint routers (wire, #2831):** a remote network's router is
  learned only when one router alone answers two of the node's
  Who-Is-Router-To-Network queries a minute apart: 60 s plus two timeouts
  after the first routed request. A replaced router is taken 60 s plus five
  timeouts after the first unanswered one (75 s at 3 s). Requests go by
  broadcast meanwhile. Answers through another router than the request's
  time out (`off_path_answer_drops`).
