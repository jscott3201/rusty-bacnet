---
section: Changed
---
- **Python API:** `export_223_experimental` reads the summary (13 ReadProperty requests without
  ReadPropertyMultiple) and the objects in batches; an object whose read failed reports `error`
  `object-name: ...`; a list that fails element by element raises, a note naming the element (#2560).
