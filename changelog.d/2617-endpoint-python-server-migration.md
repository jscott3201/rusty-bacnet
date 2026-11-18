---
section: Migration notes
---
- **Endpoint server (Python API, #2617):** replace `device_write_policy=f`
  with `mutation_policy=f`, now asked about every mutation of every object,
  subscriptions included; leave it out for a read-only server. Drop
  `services=`. Read the Device's Protocol_Services_Supported for
  `service_scope()`, and use `status()` or a server call for
  `is_session_alive()` (table in
  [docs/python-api.md](docs/python-api.md#migration-endpointserver-to-the-endpoints-bacnetserver-2617)).
