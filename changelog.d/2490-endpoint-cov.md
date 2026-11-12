---
section: Added
---
- **Wire:** an endpoint session that opts in with `EndpointSession::with_cov`
  answers SubscribeCOV and sends unconfirmed and confirmed COV notifications
  after the application's local writes, over bacnet-device-core's `Reporter`;
  without it SubscribeCOV is refused as before (#2490).
