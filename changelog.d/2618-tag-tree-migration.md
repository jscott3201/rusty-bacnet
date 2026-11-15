---
section: Migration notes
---
- **Tagged values (Python API, #2618):** where a read with a context tag gave
  `application_data`, expect `tag == "tagged"`: read the octets from
  `.octets`, not `.value`, and compare with `PropertyValue.tagged(octets)` or
  by `.octets`, since a tagged value doesn't equal `application_data` of the
  same octets. CLI JSON scripts read a value shown as its tags from
  `decoded.fields.data.raw`.
