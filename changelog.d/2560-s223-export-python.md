---
section: Changed
---
- **Python API:** `export_223_experimental` reads the summary (13 ReadProperty requests without
  ReadPropertyMultiple) and the objects in batches; an object whose read failed reports `error`
  `object-name: ...`; an Object_List that fails element by element raises (#2560).
