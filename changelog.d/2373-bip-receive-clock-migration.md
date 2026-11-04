---
section: Migration notes
---
- **B/IP receive time (Rust API, #2373):** drop the `now` argument from
  `Link::receive` calls and from your `Policy::admit_management` impl. A
  policy that needs the time, such as a rate limit, reads its own clock
  there. `Node::receive` and the ports' `receive` still take `now`.
