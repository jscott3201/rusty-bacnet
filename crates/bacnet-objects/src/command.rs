//! Command object (type 7) per ASHRAE 135-2020 Clause 12.10.
//!
//! Writing Present_Value selects one of the Action lists, each a typed
//! [`BACnetActionList`], and starts it. The object owns the bookkeeping:
//! the range check, the busy interlock while a list runs, In_Process,
//! All_Writes_Successful and each command's write-successful flag. The writes
//! themselves are the bundled server's: it takes the queued [`CommandRun`]
//! under the guard that committed the Present_Value write and makes each
//! write once that guard is released.

use crate::prelude::*;
use alloc::borrow::Cow;
use alloc::sync::Arc;
use bacnet_encoding::constructed::encode_action_list;
use bacnet_types::constructed::{BACnetActionCommand, BACnetActionList};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier, Reliability};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use bytes::BytesMut;
use core::sync::atomic::Ordering;

use crate::sync::AtomicU64;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

mod metadata;

/// The writes a Present_Value write queued, for the server to make: a
/// Command object's selected list, or a Channel object's value for each of
/// its members.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct CommandRun {
    /// The commanding object; its generation guards every report back.
    pub source: ObjectIdentifier,
    /// The generation the write started.
    pub generation: u64,
    /// What the run writes.
    pub plan: RunPlan,
    /// The objects whose runs led to this one, outermost first. The object
    /// leaves it empty; the server fills it in when one run's write starts
    /// another, so a run that would start its own object again is caught.
    pub chain: Arc<[ObjectIdentifier]>,
}

/// The next run generation. One counter serves every Command and Channel in
/// the process, so a run left over from an object that was removed never
/// matches the object that took its place.
pub(crate) fn next_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// What a [`CommandRun`] writes.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub enum RunPlan {
    /// A Command object's selected list: its commands, in the order they're
    /// made (Clause 12.10).
    Actions(Vec<BACnetActionCommand>),
    /// A Channel object's value and the members it goes to (Clause 12.53).
    Channel(crate::channel::ChannelDistribution),
}

/// Why a write a [`CommandRun`] made, or meant to make, failed. A Channel
/// reports the first failure of a run in its Reliability (Clause 12.53.9); a
/// Command keeps only whether each write succeeded.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteFailure {
    /// The target can't take the value as configured: the Channel can't
    /// coerce the value to the member's datatype, or the target says the
    /// object or property doesn't exist, or that it takes no value of this
    /// datatype.
    Configuration,
    /// The target turned the write down for another reason, or the run
    /// couldn't make it: it ended first, or the request couldn't be built or
    /// sent from here.
    Process,
    /// The target's device couldn't be reached: no binding for it, initiation
    /// disabled by DeviceCommunicationControl, or no answer to any attempt.
    Communication,
}

impl WriteFailure {
    /// The Reliability a Channel whose run failed this way reports.
    pub fn reliability(self) -> Reliability {
        match self {
            Self::Configuration => Reliability::CONFIGURATION_ERROR,
            Self::Process => Reliability::PROCESS_ERROR,
            Self::Communication => Reliability::COMMUNICATION_FAILURE,
        }
    }
}

/// BACnet Command object: runs the action list Present_Value selects.
pub struct CommandObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u64,
    in_process: bool,
    all_writes_successful: bool,
    action: Vec<BACnetActionList>,
    action_text: Option<Vec<String>>,
    status_flags: StatusFlags,
    reliability: Reliability,
    /// A fresh [`next_generation`] at every Present_Value write and Action
    /// change.
    generation: u64,
    /// The Action element (zero-based) whose commands are being made.
    running: Option<usize>,
    pending_run: Option<CommandRun>,
}

impl CommandObject {
    /// Create a new Command object with default values.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::COMMAND, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            in_process: false,
            all_writes_successful: true,
            action: Vec::new(),
            action_text: None,
            status_flags: StatusFlags::empty(),
            reliability: Reliability::NO_FAULT_DETECTED,
            generation: next_generation(),
            running: None,
            pending_run: None,
        })
    }

    /// Replace the Action array: element N is the list Present_Value N
    /// selects (Clause 12.10.8).
    ///
    /// Refused with PROPERTY / VALUE_OUT_OF_RANGE, leaving Action as it was,
    /// when a command's device identifier isn't a Device object (#1308) or a
    /// command can't be encoded: a priority outside 1..=16, or a value with no
    /// encoding. A present Action_Text follows the new size (Clause 12.10.9),
    /// keeping its leading texts and adding empty ones. A run in progress is
    /// abandoned: its remaining writes aren't made and In_Process returns to
    /// FALSE.
    pub fn set_action(&mut self, action: Vec<BACnetActionList>) -> Result<(), Error> {
        let mut scratch = BytesMut::new();
        for list in &action {
            for command in &list.commands {
                crate::device_reference::check_device_member(command.device_identifier)?;
            }
            encode_action_list(&mut scratch, list)
                .map_err(|_| common::value_out_of_range_error())?;
            scratch.clear();
        }
        if let Some(texts) = &mut self.action_text {
            texts.resize(action.len(), String::new());
        }
        self.action = action;
        self.generation = next_generation();
        self.in_process = false;
        self.running = None;
        self.pending_run = None;
        Ok(())
    }

    /// Serve Action_Text, one description per Action list (Clause 12.10.9).
    ///
    /// Refused with PROPERTY / VALUE_OUT_OF_RANGE, leaving the property as it
    /// was, unless there's exactly one text per Action element, so set Action
    /// first. The property is read-only on the network.
    pub fn set_action_text(&mut self, texts: Vec<String>) -> Result<(), Error> {
        if texts.len() != self.action.len() {
            return Err(common::value_out_of_range_error());
        }
        self.action_text = Some(texts);
        Ok(())
    }

    /// Start the list `selected` names; zero or an empty list completes at
    /// once (Clauses 12.10.5 to 12.10.8).
    fn select(&mut self, selected: u64) -> Result<(), Error> {
        // Refusing a write while a list runs also stops a Command whose list
        // writes its own Present_Value from starting over (Clause 12.10).
        if self.in_process {
            return Err(common::protocol_error(ErrorClass::OBJECT, ErrorCode::BUSY));
        }
        let index = match usize::try_from(selected) {
            Ok(0) => None,
            Ok(n) if n <= self.action.len() => Some(n - 1),
            _ => return Err(common::value_out_of_range_error()),
        };
        self.present_value = selected;
        self.generation = next_generation();
        let commands = index.map_or_else(Vec::new, |i| self.action[i].commands.clone());
        if commands.is_empty() {
            self.all_writes_successful = true;
            return Ok(());
        }
        self.in_process = true;
        self.all_writes_successful = false;
        self.running = index;
        self.pending_run = Some(CommandRun {
            source: self.oid,
            generation: self.generation,
            plan: RunPlan::Actions(commands),
            chain: Arc::from([]),
        });
        Ok(())
    }
}

impl BACnetObject for CommandObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        // Table 12-12 has no Out_Of_Service (#1064), and Clause 12.10 holds the
        // OUT_OF_SERVICE flag FALSE.
        if let Some(result) =
            read_common_properties!(self, property, array_index, no_out_of_service)
        {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::COMMAND.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.present_value))
            }
            p if p == PropertyIdentifier::IN_PROCESS => Ok(PropertyValue::Boolean(self.in_process)),
            p if p == PropertyIdentifier::ALL_WRITES_SUCCESSFUL => {
                Ok(PropertyValue::Boolean(self.all_writes_successful))
            }
            // BACnetARRAY[N] of BACnetActionList (Table 12-12): each element
            // goes out framed in its own [0] pair, so one read alone carries
            // the same octets the whole-array read concatenates.
            p if p == PropertyIdentifier::ACTION => {
                let lists = self
                    .action
                    .iter()
                    .map(|list| {
                        let mut encoded = BytesMut::new();
                        encode_action_list(&mut encoded, list)?;
                        Ok(PropertyValue::ApplicationData(encoded.to_vec()))
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                common::read_array(lists, array_index)
            }
            p if p == PropertyIdentifier::ACTION_TEXT => match &self.action_text {
                Some(texts) => common::read_array(
                    texts
                        .iter()
                        .cloned()
                        .map(PropertyValue::CharacterString)
                        .collect(),
                    array_index,
                ),
                None => Err(common::unknown_property_error()),
            },
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::PRESENT_VALUE => match value {
                PropertyValue::Unsigned(selected) => self.select(selected),
                _ => Err(common::invalid_data_type_error()),
            },
            // Action and a present Action_Text are read-only from the network;
            // unhandled_write_error answers both from the metadata.
            _ => Err(crate::common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
                array_index,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn supports_subscribe_cov_property(&self) -> bool {
        // Table 13-1 has no Command row, so SubscribeCOV stays refused. A
        // property subscription follows Table 13-1a, which lets a client watch
        // In_Process and All_Writes_Successful through a run.
        true
    }

    fn take_command_run_internal(&mut self) -> Option<CommandRun> {
        self.pending_run.take()
    }

    fn command_generation_internal(&self) -> Option<u64> {
        Some(self.generation)
    }

    fn record_command_write_internal(
        &mut self,
        generation: u64,
        command: usize,
        success: bool,
    ) -> bool {
        if generation != self.generation || !self.in_process {
            return false;
        }
        let Some(list) = self.running.and_then(|index| self.action.get_mut(index)) else {
            return false;
        };
        let Some(made) = list.commands.get_mut(command) else {
            return false;
        };
        made.write_successful = success;
        // A failed command that quits on failure ends the list: the commands
        // after it are never made and read as unsuccessful (Clause 12.10.8).
        if !success && made.quit_on_failure {
            for skipped in &mut list.commands[command + 1..] {
                skipped.write_successful = false;
            }
        }
        true
    }

    fn complete_command_run_internal(
        &mut self,
        generation: u64,
        outcome: Result<(), WriteFailure>,
    ) -> bool {
        if generation != self.generation || !self.in_process {
            return false;
        }
        self.in_process = false;
        // A failed write leaves Reliability alone (Clause 12.10.13).
        self.all_writes_successful = outcome.is_ok();
        self.running = None;
        true
    }
}

#[cfg(test)]
mod tests;
