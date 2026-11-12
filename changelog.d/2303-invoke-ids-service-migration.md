---
section: Migration notes
---
- **Reporter invoke IDs (Rust API, #2303):** implement
  `InvokeIds::take(&mut self, to, service: ConfirmedServiceChoice)`; the
  `Reporter` passes ConfirmedCOVNotification or
  ConfirmedCOVNotificationMultiple.
