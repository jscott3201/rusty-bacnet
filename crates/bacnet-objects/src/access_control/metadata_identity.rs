use super::{
    AccessCredentialObject, AccessRightsObject, AccessUserObject, CredentialDataInputObject,
};
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
};

// Canonical effective rows for the Access Identity quartet (ASHRAE 135-2020; PDF = printed + 2):
// - Access Credential (type 32, §12.35 Table 12-40; printed p. 400 / PDF p. 402)
// - Access User (type 35, §12.33 Table 12-38; printed p. 390 / PDF p. 392)
// - Access Rights (type 34, §12.34 Table 12-39; printed p. 394 / PDF p. 396)
// - Credential Data Input (type 37, §12.36 Table 12-43; printed p. 409 / PDF p. 411)
// Order preserves each legacy projection; Property_List is appended so the
// projection helper omits it while required_properties keeps it. Only
// implemented rows are described: table rows the objects do not serve stay
// absent until dispatch exists (the CDI event/intrinsic rows). Shared
// conventions match metadata_topology.rs (Slice A): OI/ON/OT
// RequiredRead/ReadOnly with the explicit Object_Name denial, Description
// Optional/Always, Out_Of_Service RequiredRead/Always on Credential Data
// Input only (Tables 12-38, 12-39 and 12-40 have none; #1064 removed the rows
// the 0.1.0 import carried), Status_Flags/Reliability RequiredRead/ReadOnly
// (table R on all four quartet tables) except the Credential Data Input
// Reliability below, writability mirroring dispatch, presence None, not
// createable but deleteable with no overrides, and Property_List plus the
// BACnetARRAY rows (the credential's two, the reader's two) array-gated.
// Table 12-40 has no Present_Value row, so the credential serves none (#979
// removed the implementation-extra row the 0.1.0 import carried).
// Credential_Status/Assigned_Access_Rights/Authentication_Factors carry the
// table R code with no write arm: the status is derived from
// Reason_For_Disable (#1073) and the two BACnetARRAYs are provisioned by the
// application, so all three are RequiredRead/ReadOnly. The rows #1073 added
// follow them, before Property_List: Global_Identifier carries the table W
// code with the routed Unsigned32 arm, so RequiredWrite/Always;
// Reason_For_Disable carries R with no arm, so RequiredRead/ReadOnly; and
// Activation_Time, Expiration_Time and Credential_Disable carry R with routed
// arms, so RequiredRead/Always. Authorization_Exemptions (#1331) carries the
// table O code with no write arm, so Optional/ReadOnly; it is a per-instance
// row, present once the application sets it, before Property_List.
// Table 12-38 has neither Present_Value nor Assigned_Access_Rights, so the
// user serves neither (#1064 removed the implementation-extra rows the 0.1.0
// import carried). User_Type/Credentials carry the table R code; the
// User_Type arm makes it RequiredRead/Always while Credentials stays
// RequiredRead/ReadOnly. User Members and Member_Of (#1394) carry the table O
// code with no write arm, so Optional/ReadOnly, after the status rows and
// before Property_List. User Global_Identifier (#1463) carries the table W
// code with the routed Unsigned32 arm Access Credential uses, so
// RequiredWrite/Always, after Member_Of. Rights Global_Identifier carries
// the table W code with the routed Unsigned arm, so RequiredWrite/Always;
// the ±rules rows carry the table R code with routed array arms (#1330), so
// RequiredRead/Always, and Enable (#1332) follows the status rows, before
// Property_List, with the table R code and a routed Boolean arm, so
// RequiredRead/Always too. Rights Accompaniment (#1393) carries the table O
// code with a routed reference arm, so Optional/Always; it is a per-instance
// row, present once the application sets it, after Enable. CDI Present_Value
// and Reliability carry the table R code with footnote 1, and dispatch takes
// their writes only while Out_Of_Service is TRUE (#1168), so
// RequiredRead/WhenOutOfService.
// Update_Time and Supported_Formats carry the table R code with no arm, so
// RequiredRead/ReadOnly; Supported_Format_Classes carries the table O code,
// so Optional/ReadOnly. Both format rows are BACnetARRAYs (#1169).
const ACCESS_CREDENTIAL_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::CREDENTIAL_STATUS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ASSIGNED_ACCESS_RIGHTS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::AUTHENTICATION_FACTORS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::GLOBAL_IDENTIFIER, RequiredWrite, None, Always),
    PropertyMetadata::new(P::REASON_FOR_DISABLE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ACTIVATION_TIME, RequiredRead, None, Always),
    PropertyMetadata::new(P::EXPIRATION_TIME, RequiredRead, None, Always),
    PropertyMetadata::new(P::CREDENTIAL_DISABLE, RequiredRead, None, Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const ACCESS_USER_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::USER_TYPE, RequiredRead, None, Always),
    PropertyMetadata::new(P::CREDENTIALS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::MEMBERS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::MEMBER_OF, Optional, None, ReadOnly),
    PropertyMetadata::new(P::GLOBAL_IDENTIFIER, RequiredWrite, None, Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const ACCESS_RIGHTS_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::GLOBAL_IDENTIFIER, RequiredWrite, None, Always),
    PropertyMetadata::new(P::POSITIVE_ACCESS_RULES, RequiredRead, None, Always),
    PropertyMetadata::new(P::NEGATIVE_ACCESS_RULES, RequiredRead, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::LOG_ENABLE, RequiredRead, None, Always),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const CREDENTIAL_DATA_INPUT_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, WhenOutOfService),
    PropertyMetadata::new(P::UPDATE_TIME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::SUPPORTED_FORMATS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::SUPPORTED_FORMAT_CLASSES, Optional, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, WhenOutOfService),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_access_credential_object(
    object: &AccessCredentialObject,
) -> Cow<'_, [PropertyMetadata]> {
    if object.authorization_exemptions().is_none() {
        return Cow::Borrowed(ACCESS_CREDENTIAL_BASE);
    }
    with_row_before_property_list(
        ACCESS_CREDENTIAL_BASE,
        PropertyMetadata::new(P::AUTHORIZATION_EXEMPTIONS, Optional, None, ReadOnly),
    )
}

pub(super) fn for_access_user_object(_object: &AccessUserObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(ACCESS_USER_BASE)
}

pub(super) fn for_access_rights_object(object: &AccessRightsObject) -> Cow<'_, [PropertyMetadata]> {
    if object.accompaniment().is_none() {
        return Cow::Borrowed(ACCESS_RIGHTS_BASE);
    }
    with_row_before_property_list(
        ACCESS_RIGHTS_BASE,
        PropertyMetadata::new(P::ACCOMPANIMENT, Optional, None, Always),
    )
}

/// `base` with a per-instance optional `row` added just before Property_List.
fn with_row_before_property_list(
    base: &[PropertyMetadata],
    row: PropertyMetadata,
) -> Cow<'static, [PropertyMetadata]> {
    let mut rows = base.to_vec();
    let before_property_list = rows
        .iter()
        .position(|row| row.property_identifier == P::PROPERTY_LIST)
        .expect("Property_List row");
    rows.insert(before_property_list, row);
    Cow::Owned(rows)
}

pub(super) fn for_credential_data_input_object(
    _object: &CredentialDataInputObject,
) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(CREDENTIAL_DATA_INPUT_BASE)
}

#[cfg(test)]
#[path = "metadata_identity_tests.rs"]
mod tests;
