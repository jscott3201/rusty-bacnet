---
section: Migration notes
---
- **Endpoint invoke IDs (Rust and Python API, #2617):** a client on an
  endpoint session with both roles holds at most 192 confirmed requests at
  once; past that a request fails at once with the invoke-IDs-exhausted
  error, where it could reach 256 while the server was idle. Keep fewer in
  flight, or give a client that needs all 256 a client-only session or a
  link of its own.
