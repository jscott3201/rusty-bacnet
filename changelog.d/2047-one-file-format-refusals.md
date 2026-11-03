---
section: Changed
---
- A stored Tags file holding a tag no write would take now fails to load as damaged data (an encoding error) rather than with the write's error code, and the colour file backend refuses to save a value out of range rather than write a file it can't load (#2047).
