---
section: Migration notes
---
- **Renumbered links (#2826):** after a link's number changes, nodes that
  learned the old one and don't follow the router's new announcement,
  standalone clients among them, keep answering with it until they
  restart; until then a client keeps its old number and counts the
  disagreement. Restart them; a server or endpoint with a configured
  Network Port number never answers with a stale one.
