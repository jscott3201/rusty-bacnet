---
section: Migration notes
---
- **Router table queries (Rust API, #2715):** a `control_authorizer` is now
  called for an Initialize-Routing-Table query from another network, whose
  new `ControlAuthContext::class` is `ControlClass::InitQuery`: return `true`
  for those to keep answering them. `ControlPolicy::Hardened` with no
  authorizer leaves them unanswered. Add `class` where `ControlAuthContext`
  is built, and `init_query` where `ControlDecisionCounters` is built or
  destructured.
