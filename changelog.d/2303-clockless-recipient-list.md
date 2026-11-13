---
section: Added
---
- With `event-reporting`, a bacnet-device-core `Responder` built without a Device clock refuses a Recipient_List write holding a destination that isn't open every day, all day, as it couldn't honour its window; the server, with the system clock, still stores them (#2303).
