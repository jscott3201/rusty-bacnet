---
section: Security
---
- A B/IP or B/IPv6 node no longer takes an answer or confirmed request from
  inside a Forwarded-NPDU, so another station cannot complete a request by
  sending the answer in a forward; counted in `forwarded_not_broadcast`.
  Broadcasts in forwards and B/IPv6 virtual addresses stay unchecked (#2829).
