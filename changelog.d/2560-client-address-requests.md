---
section: Added
---
- `BACnetClient::confirmed_request_to` and `unconfirmed_request_to` send to a
  station by its `BACnetAddress`, through the router its discovery-table row
  names, sized by that row; `bacnet_client::acks` checks a ReadProperty or
  AtomicReadFile answer as the typed calls do (#2560).
