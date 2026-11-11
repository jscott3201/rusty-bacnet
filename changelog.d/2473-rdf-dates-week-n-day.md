---
section: Added
---
- Dates also take a weekday digit or `*` after the date (`2026-11-10 2`), so
  the coming BACnet RDF encoding's date-time string parses, and a calendar
  entry takes a week-n-day as one `month,week,day` text (`11,4,4`); output is
  unchanged ([docs/CLI.md](docs/CLI.md#input-forms), #2473).
