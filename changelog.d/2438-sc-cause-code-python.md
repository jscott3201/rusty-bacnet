---
section: Added
---
- **Python API:** `ScEvent.cause_code` names an SC failure's error code, and
  a hub that closes the WebSocket with a status reads as cause
  `websocket_closed` with that status in `ScEvent.close_status` (#2438).
