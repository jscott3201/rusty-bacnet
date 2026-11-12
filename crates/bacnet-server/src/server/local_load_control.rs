//! The application's routes to a running Load Control's configuration
//! (#2497): Shed_Levels with their descriptions, Full_Duty_Baseline and
//! State_Description. Each is a typed local input ([`LocalWrite`]) made
//! through bacnet-device-core's staged local write, so the post-write work
//! is the other `*_local` routes': the event pass and the COV fanout.

use super::local_writes::LocalWrite;
use super::*;
use bacnet_objects::load_control::LoadControlSetting;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Replace a Load Control's Shed_Levels and Shed_Level_Descriptions
    /// while the server holds it, a pair for each level it can take, in
    /// order of increasing shed (Clauses 12.28.18 and 12.28.19).
    ///
    /// The two arrays are set together, so their sizes stay equal, and the
    /// number of levels may change: a network write can only rewrite the
    /// values. A level of 0 (the no-shed state, which the array does not
    /// hold) fails with PROPERTY / VALUE_OUT_OF_RANGE and changes nothing.
    /// An unknown object fails with OBJECT / UNKNOWN_OBJECT and any object
    /// other than a Load Control with OBJECT /
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, as with
    /// [`BACnetServer::set_present_value_local`].
    ///
    /// A request under way is looked at again with the new levels: a LEVEL
    /// request takes the entry the array now selects, and the object asks
    /// its [`LoadShedder`](bacnet_objects::load_control::LoadShedder) again.
    /// The clause gives no rule for a change of the array during a request
    /// (it names only writes of the four request properties as
    /// reconfiguring one, Figure 12-5), so the request neither restarts nor
    /// ends: it stays in its state unless the shedder's answer moves it.
    /// What the look changes is saved with the request, as the application's
    /// other setters' changes are. A SubscribeCOV subscriber hears of a move
    /// of the shed state (Table 13-1's Load Control row carries Present_Value,
    /// Status_Flags and the four request properties); Expected_Shed_Level and
    /// Actual_Shed_Level reach only a property subscription on them. A look
    /// that moves nothing the row carries sends no SubscribeCOV report. The
    /// route is not audited, as with `set_reliability_local`. The levels
    /// themselves are configuration, not part of the request: an object with
    /// storage does not keep them, so set them with the object's setters
    /// before it joins the server, where a restored request is put to the
    /// shedder with them as the server starts. Storage keeps only the
    /// elements a client wrote (#2502), which win over the setter's at a
    /// restart; the levels given here are the application's newer word, so
    /// they replace those, and storage drops them in a save nobody waits
    /// for. This route works only on a running server, after that first
    /// look.
    ///
    /// Like [`write_local`](Self::write_local), it must be awaited inside a
    /// Tokio runtime, failing before anything is written outside one, and a
    /// caller dropped once the write has committed skips none of the work
    /// the write owes (#1367).
    pub async fn set_load_control_shed_levels_local(
        &self,
        oid: &ObjectIdentifier,
        levels: Vec<(u64, String)>,
    ) -> Result<(), Error> {
        self.set_load_control_local(oid, LoadControlSetting::ShedLevels(levels))
            .await
    }

    /// Set a Load Control's Full_Duty_Baseline, in kilowatts, while the
    /// server holds it, or clear it with `None`; the optional property is
    /// served and listed only while one is set (Clause 12.28.15).
    ///
    /// A negative or non-finite baseline fails with PROPERTY /
    /// VALUE_OUT_OF_RANGE and changes nothing; the other errors are
    /// [`BACnetServer::set_load_control_shed_levels_local`]'s. A request
    /// under way is put to the shedder again with the new baseline
    /// ([`ShedRequest::full_duty_baseline`](bacnet_objects::load_control::ShedRequest::full_duty_baseline)),
    /// the PERCENT and AMOUNT choices being reckoned from it (Clause
    /// 12.28.10); the clause does not say that a new baseline reconfigures
    /// a request, so it stays in its state unless the shedder's answer
    /// moves it. A change of state is reported and saved as for the
    /// levels, and the route is not audited. The baseline itself is
    /// configuration, which storage does not keep: set it with the object's
    /// setter before the object joins the server.
    ///
    /// The task and drop rules are those of
    /// [`BACnetServer::set_load_control_shed_levels_local`].
    pub async fn set_load_control_full_duty_baseline_local(
        &self,
        oid: &ObjectIdentifier,
        kilowatts: Option<f32>,
    ) -> Result<(), Error> {
        self.set_load_control_local(oid, LoadControlSetting::FullDutyBaseline(kilowatts))
            .await
    }

    /// Set a Load Control's State_Description, the note for operators on
    /// its shed state (Clause 12.28.6), while the server holds it, or clear
    /// it with `None`; the optional property is served and listed only while
    /// one is set. It is read-only to clients, and fits being set from the
    /// application as the state changes.
    ///
    /// The errors are
    /// [`BACnetServer::set_load_control_shed_levels_local`]'s. The change
    /// reaches a SubscribeCOVProperty on State_Description once the
    /// database lock is released; the request is not looked at again.
    /// Clearing it sends a property subscriber nothing, since the property
    /// is gone. The route is not audited. The text is configuration, which
    /// storage does not keep.
    ///
    /// The task and drop rules are those of
    /// [`BACnetServer::set_load_control_shed_levels_local`].
    pub async fn set_load_control_state_description_local(
        &self,
        oid: &ObjectIdentifier,
        text: Option<String>,
    ) -> Result<(), Error> {
        self.set_load_control_local(oid, LoadControlSetting::StateDescription(text))
            .await
    }

    async fn set_load_control_local(
        &self,
        oid: &ObjectIdentifier,
        setting: LoadControlSetting,
    ) -> Result<(), Error> {
        self.write_local_as(
            oid,
            LocalWrite::ApplicationLoadControl(&setting),
            PropertyValue::Null,
        )
        .await
    }
}
