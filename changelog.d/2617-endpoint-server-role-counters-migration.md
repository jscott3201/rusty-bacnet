---
section: Migration notes
---
- **Endpoint server role counters (Rust API, #2617):** a monitor that read
  `PolicyCountersSnapshot::responder_declined` or the full-queue drops of a
  session with a server role reads `server_seat_counters()` for what the
  server's seat dropped, and the server's own counters (`server()`'s
  `request_admission_counters()` and the rest) for what it refused.
