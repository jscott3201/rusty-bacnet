---
section: Changed
---
- **Experimental:** the Standard 223 export maps Integer Values as it maps analog values and reads them by default (`EXPORTED_TYPES` is now `[ObjectType; 10]`), so the CLI, Python and MCP exports of a device that has them list them, at one request more each (#2456).
