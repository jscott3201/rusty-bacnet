---
section: Added
---
- A routed request that times out after its router said it was busy ends
  with `Error::RouterBusyTimeout`, still a timeout everywhere, naming the
  router and network; Python timeouts gain `busy_router` and
  `busy_network`, MCP timeouts `router_busy` (#2724).
