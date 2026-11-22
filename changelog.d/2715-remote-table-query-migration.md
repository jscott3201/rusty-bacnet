---
section: Migration notes
---
- **Router table queries (Rust API, #2715):** a `control_authorizer` is now
  called for an Initialize-Routing-Table query from another network
  (`source_net` set, no `target_nets`): return `true` for those to keep
  answering them. `ControlPolicy::Hardened` with no authorizer leaves them
  unanswered. Add `init_query` where `ControlDecisionCounters` is built or
  destructured.
