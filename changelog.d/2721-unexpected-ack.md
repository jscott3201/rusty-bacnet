---
section: Fixed
---
- **Breaking (Rust API):** a confirmed request the device acknowledges with
  the other kind of acknowledgement than its service takes (a Complex-ACK to
  WriteProperty, a Simple-ACK to ReadProperty) fails at once with the new
  `Error::UnexpectedAck` instead of completing as a success (#2721).
