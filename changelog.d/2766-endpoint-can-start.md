---
section: Added
---
- `EndpointSession::can_start` tells a refused start the session can be corrected and retried from one that ended it, and `bacnet_endpoint::StartError` re-exports the error the endpoint builders and start return (#2766).
