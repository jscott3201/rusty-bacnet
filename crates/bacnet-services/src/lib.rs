//! BACnet service request/response encode/decode per ASHRAE 135-2020.
//!
//! Each module covers one or more related BACnet services with request/response
//! structs and encode/decode functions. Service data at constructed boundaries
//! remains as raw bytes (`Vec<u8>`) — the application layer interprets
//! property values, not the service codec.

pub mod alarm_event;
pub mod alarm_summary;
pub mod audit;
pub mod common;
pub mod cov;
pub mod cov_multiple;
pub mod device_mgmt;
pub mod enrollment_summary;
pub mod file;
pub mod life_safety;
pub mod list_manipulation;
pub mod object_mgmt;
pub mod private_transfer;
pub mod read_property;
pub mod read_range;
pub mod rpm;
pub mod schedule;
pub mod text_message;
pub mod virtual_terminal;
pub mod who_am_i;
pub mod who_has;
pub mod who_is;
pub mod wpm;
pub mod write_group;
pub mod write_property;
