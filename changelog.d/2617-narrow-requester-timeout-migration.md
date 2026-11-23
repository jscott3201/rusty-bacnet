---
section: Migration notes
---
- **Endpoint client errors (Rust API, #2617):** a request the session's
  client gives up waiting for fails with an Abort `Error::Remote` whose
  reason is `TSM_TIMEOUT`, where the narrow requester returned
  `Error::Timeout`; a long request is segmented and a segmented answer
  reassembled as the identity allows (both by default), where it refused
  both.
