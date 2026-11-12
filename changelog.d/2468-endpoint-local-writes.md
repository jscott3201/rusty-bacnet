---
section: Added
---
- An application hosting its device on an endpoint makes its own changes with
  `EndpointSession::write_local`, `set_present_value_local` and
  `set_reliability_local`, saved and audited as the endpoint's network writes
  are; no COV or event notification follows them (#2468).
