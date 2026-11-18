---
section: Migration notes
---
- **Routed path limits (Rust API, #2667):** a `configure_routed_path_max_npdu` call with a limit under the APDU minimum plus the routed header now returns `Error::Encoding` instead of failing each later request; pass at least the value the error names.
