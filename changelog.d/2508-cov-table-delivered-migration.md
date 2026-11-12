---
section: Migration notes
---
- **CovTable::delivered (Rust API, #2508):** a host that runs `evaluate`
  itself evaluates again the references `delivered` returns for an
  unconfirmed report that ended unsent, as it does those an Ack returns:
  a change measured from that report found nothing then and needs a look
  against what was delivered. `bacnet-server` and the `Reporter` do this.
