---
section: Fixed
---
- An object keeping its written reporting settings no longer leaves storage
  holding rows it doesn't serve when it changes between a request's staged
  save and the write: a multi-state state count the application sets, or an
  Event_Algorithm_Inhibit the server follows (#2431).
