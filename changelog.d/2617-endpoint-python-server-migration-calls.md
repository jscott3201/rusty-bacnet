---
section: Migration notes
---
- **Endpoint server calls (Python API, #2617):** drop `services=`. Read the
  Device's Protocol_Services_Supported for `service_scope()`, and use
  `status()` or a server call for `is_session_alive()`. `status()` drops
  `responder_declined`: `server_seat_apdus` and `server_seat_full_drops`
  count what the server was handed and what its full queue dropped (table in
  [docs/python-api.md](docs/python-api.md#migration-endpointserver-to-the-endpoints-bacnetserver-2617)).
