---
section: Migration notes
---
- **Network number (wire, #2826):** a standalone client learns its
  network's number one APDU timeout after its What-Is-Network-Number, from
  the answers that agree, configured answers outranking learned ones. A
  router's announcement at its start now draws one question first. Answers
  naming two numbers leave the number as it was and raise
  `BACnetClient::network_number_disagreements`.
