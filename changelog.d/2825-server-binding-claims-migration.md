---
section: Migration notes
---
- **Server device bindings (wire, #2825):** a first write or notification to
  a device waits for the end of the server's Who-Is, and nothing binds when
  two stations answer (`BACnetServer::device_binding_conflicts`); the
  notifications that waited are dropped and counted in
  `device_recipient_unbound`. A device that moved is followed about two
  minutes after it announces its new address (84 s by default). Configure a
  `DeviceBinding` for a device that only announces itself or whose alarms
  can't wait.
