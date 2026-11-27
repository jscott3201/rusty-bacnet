---
section: Migration notes
---
- **Server device bindings (wire, #2825):** a first write or notification to
  a device waits for the end of the server's Who-Is, and nothing binds when
  two stations answer (`device_binding_conflicts`); notifications that waited
  are dropped and counted in `device_recipient_unbound`. A device that moved
  is followed 2 x (retries + 1) x timeout + 60 s after it announces its new
  address: 84 s at the 3 s default timeout, 108 s at 6 s, with 3 retries.
