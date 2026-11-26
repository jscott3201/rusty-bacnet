---
section: Changed
---
- **Python API:** a watch asks again for a subscription that went
  unanswered, met a full table or failed its renewal after 5 minutes, as
  Rust's watches do, not after 30 s doubling to 5 minutes (#2603).
