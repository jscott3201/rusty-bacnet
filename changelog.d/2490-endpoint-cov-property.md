---
section: Added
---
- **Wire:** with `with_cov`, an endpoint session also answers and advertises
  SubscribeCOVProperty, and `BipEndpointBuilder::cov` turns COV on from the
  builder; without the opt-in, SubscribeCOVProperty is refused as before
  (#2490).
