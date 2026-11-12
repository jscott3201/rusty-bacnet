---
section: Fixed
---
- **Python API:** `on_reinitialize` and `mutation_policy` answers are now read under the same refusal as the callable, so user code run while reading them (an awaitable's `close()`) cannot re-enter the bindings (#2498).
