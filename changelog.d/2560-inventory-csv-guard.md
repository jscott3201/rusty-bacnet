---
section: Security
---
- The inventory's CSV writes a name, description or state text that a spreadsheet would run as a
  formula (it starts with `=`, `+`, `-`, `@`, a tab or a carriage return) behind an apostrophe;
  `CsvOptions::with_exact` and `bacnet inventory --csv-exact` turn the guard off (#2560).
