---
section: Added
---
- **Python API (experimental):** `BACnetClient.export_223_experimental()`
  reads a device's points into an `Inventory223Experimental`, whose
  `to_turtle()` writes the same Standard 223 point list as the CLI
  ([docs](docs/python-api.md#standard-223-export-experimental), #2436).
