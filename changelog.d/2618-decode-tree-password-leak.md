---
section: Security
---
- Since 0.15.0 the decode tree of a DeviceCommunicationControl or
  ReinitializeDevice request its decoder refused could show numbers read
  from the password's octets: opening-tag labels' tag numbers and
  trailing-octet counts, after a password that claims too few (#2618).
