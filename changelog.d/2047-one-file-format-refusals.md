---
section: Changed
---
- Tags no write would take now fail to load as an encoding error, from a file or a custom backend. The colour, Notification Class, forwarder and Access Rights file backends refuse to save what they couldn't load: an out-of-range value, an over-long list or an oversized file (#2047).
