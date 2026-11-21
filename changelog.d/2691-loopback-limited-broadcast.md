---
section: Fixed
---
- **B/IP discovery.** An explicit loopback interface on a wildcard socket refuses local limited broadcasts before they can reach a LAN and make the device discover itself. Configure the loopback subnet's broadcast address to discover on loopback (#2691).
