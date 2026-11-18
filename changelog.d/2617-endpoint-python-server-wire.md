---
section: Changed
---
- **Breaking (wire):** a Python endpoint answers Who-Is, Who-Has and every
  service a standalone server executes, where it rejected all but
  ReadProperty and its opt-in Device WriteProperty and ReinitializeDevice;
  its Device lists that set, with services the read-only server denies
  (#2617).
