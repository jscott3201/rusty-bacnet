---
section: Migration notes
---
- **B/IPv6 foreign-device one-shots (CLI, #2388):** a command run with `--ipv6 --foreign-bbmd` now waits for the BBMD to accept its registration and exits with an error if it refuses or stays silent for 5 seconds. Fix the BBMD's address or its foreign-device policy, or drop `--foreign-bbmd` to run on the local link only.
