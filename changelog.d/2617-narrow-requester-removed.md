---
section: Removed
---
- **Breaking (Rust API):** The endpoint's narrow requester is removed:
  `ClientRoleHandle`, `bacnet_endpoint::roles` with `Commandability`,
  `cloned_client_handle` and the `EndpointApduDestination` re-export.
  `EndpointSession::client()` returns the full `BACnetClient` (#2617).
