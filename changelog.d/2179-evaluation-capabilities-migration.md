---
section: Migration notes
---
- **ReliabilityEval (Rust API, #2179):** move `evaluate_reliability_internal`,
  `reliability_evaluation_inhibited_internal` and `set_reliability_internal`
  into `impl ReliabilityEval`, dropping `_internal`, and return `Some(self)`
  from `reliability_eval()`; `obj.set_reliability_internal(r)` becomes
  `obj.reliability_eval()?.set_reliability(r)`. Seed Event_State and
  Acked_Transitions through `EventReporting::commit_event_transition`.
