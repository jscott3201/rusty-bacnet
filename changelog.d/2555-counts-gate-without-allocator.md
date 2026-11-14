---
section: Changed
---
- The counting harness judges each row without the system allocator's work (its functions, the copy a moving `realloc` makes and its atomic helpers), which moved rows by hundreds of instructions per request on heap layout alone; it reports that work separately, and allocation calls stay gated (#2555).
