---
section: Changed
---
- **Wire (bacnet-device-core):** with `event-reporting`, a `Responder` built without a Device clock refuses a Recipient_List write holding a destination that isn't open every day, all day, with PROPERTY / VALUE_OUT_OF_RANGE; the server still stores them (#2303).
