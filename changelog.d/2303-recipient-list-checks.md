---
section: Added
---
- `bacnet_objects::notification_class::decode_recipient_list` and `bacnet_device_core::write::check_clockless_recipients`, for a host that judges Recipient_List destinations before a class takes them, such as one running AddListElement itself (#2303).
