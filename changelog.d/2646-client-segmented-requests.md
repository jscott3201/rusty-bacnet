---
section: Fixed
---
- **Wire:** A client that receives segments reassembles a segmented confirmed request instead of
  aborting it, then answers as it does an unsegmented one; a requester's Abort ends the transfer.
  Beside a server role the client takes only COV notifications this way (#2646).
