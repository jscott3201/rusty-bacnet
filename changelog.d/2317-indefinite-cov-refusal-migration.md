---
section: Migration notes
---
- **SubscribeCOV refusal (wire, #2317):** a client that recognised a refused
  indefinite subscription by SERVICES / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED
  now gets SERVICES / VALUE_OUT_OF_RANGE; ask again with a finite lifetime.
