---
section: Changed
---
- **Breaking (Rust API):** a request to an empty MAC, by
  `confirmed_request(&[], ..)`, `unconfirmed_request(&[], ..)` or any typed
  call, is refused before anything is sent, with the error an address on
  network 0 with no MAC gets; before, it reached the transport (#2588).
