---
section: Migration notes
---
- **Endpoint server calls (Python API, #2617):** drop `services=`. Read the
  Device's Protocol_Services_Supported for `service_scope()`, and use
  `status()` or a server call for `is_session_alive()` (table in
  [docs/python-api.md](docs/python-api.md#migration-endpointserver-to-the-endpoints-bacnetserver-2617)).
