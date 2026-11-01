---
section: Added
---
- Applications can register a CreateObject constructor per object type
  (Device and Network Port excepted) with the server, making their own
  proprietary types creatable over the network or replacing a built-in
  type's defaults; the PICS reports a registered type as createable (#2173).
