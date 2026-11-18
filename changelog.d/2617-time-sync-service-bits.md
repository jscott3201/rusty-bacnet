---
section: Fixed
---
- **Wire:** a server whose time-synchronization policy is off, or accepts no source, no longer lists TimeSynchronization and UTCTimeSynchronization in the Device's Protocol_Services_Supported; one that accepts a source lists both. A Group's ReadRange now serves that list as ReadProperty does (#2617).
