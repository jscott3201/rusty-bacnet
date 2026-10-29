//! Profile entry decisions happen before any filesystem or network effects.

use super::{Overrides, overrides::has_transport_flags, service::Service, store::Store};
use crate::args::{Cli, Command};
use crate::tui::app::connection::Connection;

/// Explicit flags without a profile, --no-profile, or the MS/TP monitor,
/// never inspect the store.
pub(crate) fn enabled(cli: &Cli) -> bool {
    matches!(&cli.command, Some(Command::Tui { profile, no_profile: false, mstp_monitor: None, .. })
        if profile.is_some() || !has_transport_flags(cli))
}

/// Open the profile service. A flags-only run returns `None` without calling
/// `discover`, so it reads and creates nothing in the config dir.
pub(crate) fn open(
    cli: &Cli,
    discover: impl FnOnce() -> Result<Store, String>,
) -> Result<Option<(Service, Connection)>, String> {
    if !enabled(cli) {
        return Ok(None);
    }
    let profile_name = match &cli.command {
        Some(Command::Tui { profile, .. }) => profile.as_deref(),
        _ => None,
    };
    let overrides = Overrides::from_cli(cli)?;
    let service = Service::new(discover(), overrides);
    let connection = select(&service, profile_name);
    Ok(Some((service, connection)))
}

/// Zero profiles opens creation; one opens its preview; many show a picker.
/// Listing/loading failures remain visible and recoverable in the dialog.
pub(crate) fn select(service: &Service, name: Option<&str>) -> Connection {
    let mut connection = Connection::new(service.overrides.clone());
    connection.reply(service.catalog());
    if let Some(name) = name {
        connection.reply(service.load(name));
    } else if connection.error.is_none() {
        match connection.names.as_slice() {
            [name] => connection.reply(service.load(&name.clone())),
            [] => connection.new_profile(),
            _ => {}
        }
    }
    connection
}
