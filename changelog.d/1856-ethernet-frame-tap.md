---
section: Added
---
- The Ethernet transport has a frame tap: `EthernetTransport::frame_tap()`,
  also reached through `AnyTransport::frame_tap()`, reports every 802.3 frame
  it sends and every one it takes, XID and TEST included (#1856).
