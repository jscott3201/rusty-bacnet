---
section: Changed
---
- **Binding:** on Linux a B/IP multicast node that starts with its group on
  its own port binds its interface address, with broadcast listeners, so it
  can claim the port beside the group's listener; its broadcast address must
  be its subnet's or 255.255.255.255 (#2366).
