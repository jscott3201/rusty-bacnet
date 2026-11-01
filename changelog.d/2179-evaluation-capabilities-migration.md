---
section: Migration notes
---
- **ReliabilityEval (Rust API, #2179):** move `evaluate_reliability_internal`,
  `reliability_evaluation_inhibited_internal` and `set_reliability_internal`
  into `impl ReliabilityEval` without `_internal`, lent by `reliability_eval()`.
  Calls become `obj.reliability_eval().map(|r| r.set_reliability(..))` and
  `obj.reliability_eval().map(|r| r.evaluate_reliability())`
  ([details](docs/rust-api.md#capabilities)).
