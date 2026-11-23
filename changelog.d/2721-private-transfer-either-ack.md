---
section: Changed
---
- ConfirmedPrivateTransfer takes either acknowledgement on the raw request,
  as vendor devices answer it both ways; the typed call reports a
  Simple-ACK as `Error::UnexpectedAck`, since it carries no vendor or
  service number (#2721).
