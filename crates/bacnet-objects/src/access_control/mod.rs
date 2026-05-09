//! Access Control objects (ASHRAE 135-2020 Clause 12).
//!
//! This module implements the seven BACnet access control object types:
//! - AccessDoor (type 30)
//! - AccessCredential (type 32)
//! - AccessPoint (type 33)
//! - AccessRights (type 34)
//! - AccessUser (type 35)
//! - AccessZone (type 36)
//! - CredentialDataInput (type 37)

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

// ---------------------------------------------------------------------------

mod credential;
mod credential_data_input;
mod door;
mod point;
mod rights;
mod user;
mod zone;
pub use credential::*;
pub use credential_data_input::*;
pub use door::*;
pub use point::*;
pub use rights::*;
pub use user::*;
pub use zone::*;

#[cfg(test)]
mod tests;
