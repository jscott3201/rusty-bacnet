---
section: Changed
---
- **Breaking (wire):** a router drops an Initialize-Routing-Table update
  that carries an SNET, so one from another network, unless
  `RouterOptions::remote_table_updates()` (`Options::remote_table_updates`
  in `bacnet-router-core`) is on. Local updates and table queries are
  unchanged (#2717).
