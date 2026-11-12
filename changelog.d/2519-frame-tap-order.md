---
section: Fixed
---
- The B/IP, B/IPv6, BACnet/SC and Ethernet frame taps report a sent frame just
  before the socket writes it, so a capture never shows a reply ahead of its
  request; a frame whose send then fails is reported too (#2519).
