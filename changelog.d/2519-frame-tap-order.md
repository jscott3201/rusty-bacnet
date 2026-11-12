---
section: Fixed
---
- **Rust API:** The B/IP, B/IPv6, BACnet/SC and Ethernet frame taps report a
  sent frame just before the socket writes it, so a reply never shows ahead of
  its request; a frame whose write then fails or is abandoned is now reported
  too, where before it was not (#2519).
