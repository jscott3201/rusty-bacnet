---
section: Added
---
- Every `.mcpb` MCP bundle carries a 512x512 project icon. The release
  workflow can sign the bundles, but signing stays off until the maintainer
  adds a code-signing certificate to the release environment's secrets; until
  then they ship unsigned, as before (#2324).
