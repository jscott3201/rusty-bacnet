//! The command-run capability: the writes a Command or Channel object's
//! Present_Value write queues.

use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;

use crate::channel::MemberDatatype;
use crate::command::{CommandRun, WriteFailure};

/// A Command's action lists or a Channel's member writes, run by the host.
///
/// The object returns itself from
/// [`BACnetObject::command_runner_mut`](crate::traits::BACnetObject::command_runner_mut),
/// and from [`command_runner`](crate::traits::BACnetObject::command_runner)
/// for the generation check a host makes under a shared database guard.
/// The bundled server takes the run under the guard that committed the
/// Present_Value write, makes the writes after releasing it and records
/// each outcome under a fresh guard. Not public API yet (#2179).
#[doc(hidden)]
pub trait CommandRunner {
    /// Take the run the last Present_Value write queued: a Command's
    /// selected list, or a Channel's value for its members.
    fn take_command_run(&mut self) -> Option<CommandRun>;

    /// The current run generation. A run whose generation no longer matches
    /// is stale and makes no further writes.
    fn command_generation(&self) -> u64;

    /// Record how command `command` of a Command's running list fared.
    ///
    /// Returns whether the run is still the current one; a stale generation
    /// is ignored, so older work can't mark a newer run. The default, for a
    /// Channel, records nothing.
    fn record_command_write(&mut self, _generation: u64, _command: usize, _success: bool) -> bool {
        false
    }

    /// Keep `datatype`, learned for a Channel's member in another device, for
    /// the distributions after this one, or forget the one kept with `None`:
    /// the member at zero-based `slot` of List_Of_Object_Property_References,
    /// if that slot still holds `reference`. Writing the member, or the whole
    /// list, forgets it too. The default, for a Command, keeps nothing.
    fn remember_member_datatype(
        &mut self,
        _slot: usize,
        _reference: &BACnetDeviceObjectPropertyReference,
        _datatype: Option<MemberDatatype>,
    ) {
    }

    /// End the current run, setting a Command's All_Writes_Successful or a
    /// Channel's Write_Status and Reliability. `outcome` is `Ok` when every
    /// write was made and succeeded, otherwise the run's first failure.
    ///
    /// Returns whether readable state changed; a stale generation is ignored.
    fn complete_command_run(&mut self, generation: u64, outcome: Result<(), WriteFailure>) -> bool;
}
