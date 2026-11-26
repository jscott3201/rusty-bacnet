---
section: Security
---
- B/IP and B/IPv6 nodes no longer take an acknowledgment, error, abort or
  confirmed request from inside a Forwarded-NPDU, whose origin no one checks,
  so a station on the link can no longer answer a request as another device;
  each such forward is counted in `forwarded_not_broadcast` (#2829).
