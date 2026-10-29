---
section: Security
---
- Captures mask DeviceCommunicationControl and ReinitializeDevice passwords in
  every frame they keep, through the end of the request and in segmented
  requests, including received and damaged frames, failing closed when a frame
  can't be read cleanly (#2062, #2074, #2084).
