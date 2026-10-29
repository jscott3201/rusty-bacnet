---
section: Added
---
- `StartedRouter::into_application_port` makes the router's own application a
  transport for a `BACnetClient`, and `into_application_ports` splits it
  between a `BACnetServer` and a client; the ports follow the router ports'
  NPDU limits as they change (#1808, #2069, #2070, #2087).
