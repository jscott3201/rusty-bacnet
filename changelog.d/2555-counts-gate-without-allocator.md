---
section: Changed
---
- The counting harness judges each row without the system allocator's own instructions, which moved rows by hundreds per request on heap layout alone; it reports them in a column of their own, and allocation calls stay gated (#2555).
