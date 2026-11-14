---
section: Changed
---
- `bacnet write` refuses a value whose `@N` differs from `--priority`,
  sending nothing, instead of writing at `--priority`, and refuses a
  `--priority` outside 1 to 16 as it reads its arguments (#2560).
