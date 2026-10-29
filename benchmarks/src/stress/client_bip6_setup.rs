//! Retain partial B/IPv6 setup owners for the caller's ordinary cleanup.
use super::StepReport;
use bacnet_types::error::Error;
use std::net::SocketAddrV6;

pub(super) async fn setup<C>(
    address: Result<SocketAddrV6, Error>,
    count: u64,
    clients: &mut Vec<C>,
    mut build: impl AsyncFnMut() -> Result<C, Error>,
    mut resolve: impl AsyncFnMut(&C, SocketAddrV6) -> Result<Vec<u8>, Error>,
    report: &mut StepReport,
) -> Option<Vec<u8>> {
    let address = match address {
        Ok(address) => address,
        Err(error) => {
            report.diagnostics.fail("server_setup", None, error);
            return None;
        }
    };
    let mut mac = Vec::new();
    for _ in 0..count {
        let participant = clients.len();
        let result = async {
            clients.push(build().await?);
            resolve(clients.last().unwrap(), address).await
        }
        .await;
        match result {
            Ok(resolved) => mac = resolved,
            Err(error) => {
                report.diagnostics.started_clients = clients.len();
                report
                    .diagnostics
                    .fail("client_setup", Some(participant), error);
                return None;
            }
        }
    }
    Some(mac)
}

#[cfg(test)]
#[path = "client_bip6_setup_tests.rs"]
mod tests;
