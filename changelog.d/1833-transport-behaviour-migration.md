---
section: Migration notes
---
- **Transport I/O and length errors (Rust and Python API, #1833):** match
  `Error::Transport` for a serial port's open, read or write, GPIO, the RS-485
  ioctl, VMAC entropy and the SC direct bind, and `Error::SendTooLong` for an
  NPDU too long for B/IP, MS/TP or Ethernet; a link's refusal of a
  routing-table query is `RoutingTableError::Send`. In Python, catch
  `OSError` where a serial open raised `RuntimeError`, and `BacnetError`
  where `routing_table()` raised `ValueError`.
