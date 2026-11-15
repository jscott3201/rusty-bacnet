---
section: Migration notes
---
- **Tagged values (Python API, #2618):** a read with a context tag may be
  `"tagged"` or, holding something `PropertyValue` can't or over 10,000
  elements, still `"application_data"`: accept both, read octets from
  `.octets`, and compare by `.octets`, since a value written as
  `application_data` reads back `tagged` and unequal. A failed typed read
  is `tagged`. CLI JSON scripts read `decoded.fields.data.raw`.
