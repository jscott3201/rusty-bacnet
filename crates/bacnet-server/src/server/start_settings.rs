//! What a server's start checks of its settings before it changes its
//! database or starts a link (#2617). Every start runs [`settle`]; a node
//! runs it too, on the settings its seat will give the server, before its
//! own link starts, so a refusal there leaves the node as it was. One
//! function for both, so the two can't drift.
use super::super::request_tasks::RequestTasks;
use super::super::*;

/// What a start goes on with once its settings are checked.
pub(in crate::server) struct StartSettings {
    /// The configured routes, checked against the link's group rule.
    pub(in crate::server) device_bindings: DeviceBindingTable,
    /// The Device's segment timeout (Tseg): a segmented request whose
    /// requester is silent for four of them ends.
    pub(in crate::server) segment_timeout: Duration,
    /// The request owner, with every policy and budget it holds checked.
    pub(in crate::server) request_tasks: Arc<RequestTasks>,
}

/// What a start checks before it changes `db` or starts its link: the
/// Device's Max_APDU_Length_Accepted against `transport`, fixed here for the
/// server's life (a later fall in the transport's capacity leaves it as is,
/// and each reply is sized to its route instead, #2093); [`settle`] with the
/// link's group rule; and `db`'s Device against the identity.
pub(in crate::server) fn check<T: TransportPort>(
    config: &mut ServerConfig,
    db: &ObjectDatabase,
    configured: Vec<DeviceBinding>,
    transport: &T,
) -> Result<StartSettings, Error> {
    super::super::network_port::validate_apdu_capacity(config, transport)?;
    let settled = settle(config, configured, |mac| {
        transport.is_group_destination(mac)
    })?;
    crate::local_device::validate_startup(db, &config.device)?;
    Ok(settled)
}

/// Check `config` and put its Audit Reporters in order; check `configured`
/// against the link's group rule `is_group`; check the Device's segment
/// timeout and every policy and budget the request owner holds. A Device
/// storage path no builder took is refused (#2730).
pub(in crate::server) fn settle(
    config: &mut ServerConfig,
    configured: Vec<DeviceBinding>,
    is_group: impl Fn(&[u8]) -> bool,
) -> Result<StartSettings, Error> {
    crate::server::device_storage::refuse_unused_path(config, "a database a start is given")?;
    if let Some(profile) = &mut config.audit_reporters {
        profile.canonicalize()?;
    }
    let device_bindings = DeviceBindingTable::from_configured(configured, is_group)?;
    let segment_timeout = super::super::segmented_receive::validate_segment_timeout(config)?;
    let request_tasks = RequestTasks::for_server(config)?;
    Ok(StartSettings {
        device_bindings,
        segment_timeout,
        request_tasks,
    })
}
