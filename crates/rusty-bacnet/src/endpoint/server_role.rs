//! The endpoints' server keywords (#2617): what each start builds the
//! session's full server from. `BipEndpoint`, `ScEndpoint` and
//! `MstpEndpoint` take the same ones, parsed once at construction as
//! `BACnetServer`'s keywords of the same names are, and kept for the build
//! at each start.
//!
//! Without `mutation_policy` the server takes the read-only stance of
//! [`bacnet_endpoint::session::read_only_server`]: no peer writes, creates
//! or deletes, so an endpoint nobody gave a policy is never writable by a
//! peer. With one, it is the server `BACnetServer(mutation_policy=...)` is,
//! with that policy as given. Time synchronization is off on every
//! endpoint unless `time_sync_policy` turns it on, whatever the mutation
//! policy (maintainer decision on #2617): unlike `BACnetServer()`, whose
//! default takes every valid request.

use bacnet_device_core::cov::CovPolicy;
use bacnet_device_core::read::ReadPropertyMultipleBudget;
use bacnet_server::mutation::MutationPolicy;
use bacnet_server::server::{BACnetServer, ServerBuilder, TimeSyncPolicy};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::app_transport::AppTransport;
use crate::endpoint::running::Session;
use crate::mutation_policy::MutationSetting;
use crate::reinitialize::EndpointReinitialize;

/// An endpoint's validated server keywords.
#[derive(Clone)]
pub(crate) struct ServerKeywords {
    /// `mutation_policy`, or `None` for the read-only stance.
    mutation: Option<MutationSetting>,
    /// `read_work_limit`: the server's ReadPropertyMultiple budget's result
    /// elements, which bound a ReadProperty's Group expansion too.
    read_work_limit: usize,
    /// `cov_policy`, read as `BACnetServer`'s is.
    cov_policy: CovPolicy,
    /// `time_sync_policy`, read as `BACnetServer`'s is; off when not given.
    time_sync: TimeSyncPolicy,
    /// `on_reinitialize`, `after_reinitialize` and `reinit_password`.
    reinitialize: EndpointReinitialize,
}

impl ServerKeywords {
    /// Parse the keywords, refusing what `BACnetServer`'s constructor
    /// refuses in the same ones, and `read_work_limit=0`. A
    /// `time_sync_policy` of `None` is time synchronization off; a dict is
    /// read as `BACnetServer`'s, so `{}` takes every valid request.
    pub(crate) fn parse(
        mutation_policy: Option<&Bound<'_, PyAny>>,
        read_work_limit: usize,
        cov_policy: Option<&Bound<'_, PyDict>>,
        time_sync_policy: Option<&Bound<'_, PyDict>>,
        on_reinitialize: Option<&Bound<'_, PyAny>>,
        after_reinitialize: Option<&Bound<'_, PyAny>>,
        reinit_password: Option<String>,
    ) -> PyResult<Self> {
        let reinitialize =
            EndpointReinitialize::parse(on_reinitialize, after_reinitialize, reinit_password)?;
        let mutation = mutation_policy
            .filter(|value| !value.is_none())
            .map(crate::mutation_policy::parse)
            .transpose()?;
        Ok(Self {
            mutation,
            read_work_limit: super::common::parse_read_work_limit(read_work_limit)?,
            cov_policy: crate::server::cov_policy::cov_policy(cov_policy)?,
            time_sync: match time_sync_policy {
                Some(dict) => crate::server::time_sync_policy::time_sync_policy(Some(dict))?,
                None => TimeSyncPolicy {
                    enabled: false,
                    ..TimeSyncPolicy::default()
                },
            },
            reinitialize,
        })
    }

    /// Whether the server's policy decides every mutation, as the session
    /// needs beside a source Audit Reporter, so that no peer rewrites the
    /// Device's Audit recipient unasked: the read-only stance, `"deny_all"`
    /// and a callable do, `"permissive"` doesn't.
    pub(crate) fn decides_mutations(&self) -> bool {
        self.mutation.as_ref().is_none_or(|setting| {
            setting.mode == MutationPolicy::DenyAll || setting.callback.is_some()
        })
    }

    /// See [`crate::reinitialize::PyReinitialize::bind_event_loop`]; called
    /// by `start()`.
    pub(crate) fn bind_event_loop(&self, py: Python<'_>) -> PyResult<()> {
        self.reinitialize.bind_event_loop(py)
    }

    /// Have `session`'s start build its full server from these keywords.
    pub(crate) fn install(&self, session: Session) -> Session {
        session.with_server(self.builder())
    }

    /// The server builder for one start.
    fn builder(&self) -> ServerBuilder<AppTransport> {
        let builder = match &self.mutation {
            None => bacnet_endpoint::session::read_only_server(),
            Some(setting) => {
                let builder = BACnetServer::generic_builder().mutation_policy(setting.mode);
                match crate::mutation_policy::server_policy(false, setting.callback.clone()) {
                    Some(policy) => builder.policy(policy),
                    None => builder,
                }
            }
        };
        let builder = builder
            .read_property_multiple_budget(ReadPropertyMultipleBudget {
                max_result_elements: self.read_work_limit,
                ..ReadPropertyMultipleBudget::default()
            })
            .cov_policy(self.cov_policy.clone())
            .time_sync_policy(self.time_sync.clone());
        self.reinitialize.install(builder)
    }
}
