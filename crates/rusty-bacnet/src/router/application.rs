//! A client, and optionally a server, on the router's own application
//! entity (#2071).
//!
//! The router's `application=` keyword picks the share at construction:
//! the whole entity for one client, or a client and a server. Each start
//! makes fresh ports and parks them in the session, and the stack built
//! with `router=` takes its own when it starts. A port goes once per router
//! start: stopping the router ends both ports, and a restarted router makes
//! new ones.

use super::lifecycle::Lifecycle;
use super::session::NativeSession;
use bacnet_network::router::{
    BACnetRouter, RouterApplicationPort, RouterApplicationPorts, StartedRouter,
};
use bacnet_transport::port::TransportPort;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::sync::{Mutex, PoisonError};

/// The longest APDU or segment wait a stack on a split entity may use: the
/// ports forget a request 60 seconds after its last segment, retry or
/// answer (see `RouterApplicationPorts`).
pub(crate) const SPLIT_TIMEOUT_LIMIT_MS: u64 = 60_000;

/// How the router shares its application entity, fixed at construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Application {
    /// No local application: the router only forwards.
    None,
    /// One client takes everything the router delivers to its application.
    Client,
    /// A client and a server split it.
    ClientAndServer,
}

impl Application {
    /// The `application=` keyword.
    pub(super) fn parse(value: Option<&str>) -> PyResult<Self> {
        match value {
            None => Ok(Self::None),
            Some("client") => Ok(Self::Client),
            Some("client_and_server") => Ok(Self::ClientAndServer),
            Some(other) => Err(PyValueError::new_err(format!(
                "application must be None, 'client' or 'client_and_server', not {other:?}"
            ))),
        }
    }

    /// The keyword's value, for messages.
    pub(super) fn name(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Client => Some("client"),
            Self::ClientAndServer => Some("client_and_server"),
        }
    }
}

/// Which stack a port is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// A `BACnetClient`.
    Client,
    /// A `BACnetServer`.
    Server,
}

impl Role {
    fn name(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Server => "server",
        }
    }
}

/// The ports one router start made and no stack has taken yet. A leaf lock,
/// never held across an await.
#[derive(Default)]
pub(super) struct Slots {
    ports: Mutex<(Option<RouterApplicationPort>, Option<RouterApplicationPort>)>,
}

impl Slots {
    /// Split `started` as `application` asks, parking the ports.
    pub(super) fn split(application: Application, started: StartedRouter) -> (BACnetRouter, Self) {
        let (router, ports) = match application {
            Application::None => {
                // A forwarding owner: the router drops what it would deliver.
                drop(started.apdus);
                (started.router, (None, None))
            }
            Application::Client => {
                let (router, port, _controls) = started.into_application_port();
                (router, (Some(port), None))
            }
            Application::ClientAndServer => {
                let (router, ports, _controls) = started.into_application_ports();
                let RouterApplicationPorts { client, server, .. } = ports;
                (router, (Some(client), Some(server)))
            }
        };
        (
            router,
            Self {
                ports: Mutex::new(ports),
            },
        )
    }

    /// Take `role`'s port, once.
    fn take(&self, role: Role) -> Option<RouterApplicationPort> {
        self.with(role, Option::take)
    }

    /// The receive capacity `role`'s port declares, leaving it parked.
    fn capacity(&self, role: Role) -> Option<u16> {
        self.with(role, |port| {
            port.as_ref()
                .map(TransportPort::local_receive_apdu_capacity)
        })
    }

    fn with<R>(&self, role: Role, f: impl FnOnce(&mut Option<RouterApplicationPort>) -> R) -> R {
        let mut ports = self.ports.lock().unwrap_or_else(PoisonError::into_inner);
        match role {
            Role::Client => f(&mut ports.0),
            Role::Server => f(&mut ports.1),
        }
    }
}

/// What a `BACnetClient` or `BACnetServer` built with `router=` keeps: the
/// router's lifecycle, not the Python object, so the stack never keeps a
/// dropped router alive.
#[derive(Clone)]
pub(crate) struct RouterApplication {
    owner: Lifecycle<NativeSession>,
    role: Role,
}

impl RouterApplication {
    /// Take this stack's port from the running router.
    ///
    /// Raises `RuntimeError` when the router isn't running, or is closing,
    /// and when this router start's port for the role is already taken.
    pub(crate) async fn take(&self) -> PyResult<RouterApplicationPort> {
        let role = self.role;
        self.found(self.owner.inspect(|session| session.slots.take(role)).await)
    }

    /// The receive capacity this stack's port declares, without taking it,
    /// so a server validates its database before the port is spent. Raises
    /// as [`Self::take`] does.
    pub(crate) async fn capacity(&self) -> PyResult<u16> {
        let role = self.role;
        self.found(
            self.owner
                .inspect(|session| session.slots.capacity(role))
                .await,
        )
    }

    fn found<T>(&self, lookup: Option<Option<T>>) -> PyResult<T> {
        let role = self.role.name();
        match lookup {
            None => Err(PyRuntimeError::new_err(format!(
                "router not started or already closing: start the router before the {role} on \
                 its application entity"
            ))),
            Some(None) => Err(PyRuntimeError::new_err(format!(
                "this router start's application {role} port is already taken: one {role} runs \
                 on it per router start, so restart the router to start another"
            ))),
            Some(Some(found)) => Ok(found),
        }
    }
}

impl super::PyBACnetRouter {
    /// The handle a stack with `router=` keeps, checked at the stack's
    /// construction: the router must share its entity with `role`.
    pub(crate) fn application_for(&self, role: Role) -> PyResult<RouterApplication> {
        let allowed = match role {
            Role::Client => self.application != Application::None,
            Role::Server => self.application == Application::ClientAndServer,
        };
        if !allowed {
            let wanted = match role {
                Role::Client => "application='client' or 'client_and_server'",
                Role::Server => "application='client_and_server'",
            };
            let actual = self
                .application
                .name()
                .map_or_else(|| "no application".to_string(), |name| format!("'{name}'"));
            return Err(PyValueError::new_err(format!(
                "a {} on the router needs BACnetRouter(..., {wanted}); this router has {actual}",
                role.name()
            )));
        }
        Ok(RouterApplication {
            owner: self.lifecycle.clone(),
            role,
        })
    }

    /// Whether the router splits its entity, so each stack's APDU and
    /// segment timeouts must stay under [`SPLIT_TIMEOUT_LIMIT_MS`].
    pub(crate) fn is_split(&self) -> bool {
        self.application == Application::ClientAndServer
    }
}

/// The `router=` keyword of `BACnetClient` and `BACnetServer`: given exactly
/// when `transport='router'`, and on a split entity with `timeout_ms` (the
/// stack's APDU or segment timeout) under 60 seconds. `ValueError` before
/// any I/O otherwise.
pub(crate) fn stack_router(
    transport: &str,
    router: Option<PyRef<'_, super::PyBACnetRouter>>,
    role: Role,
    timeout_ms: u64,
    timeout_name: &str,
) -> PyResult<Option<RouterApplication>> {
    match (transport, router) {
        ("router", None) => Err(PyValueError::new_err(
            "transport='router' needs router=BACnetRouter(...)",
        )),
        ("router", Some(router)) => {
            if router.is_split() && timeout_ms >= SPLIT_TIMEOUT_LIMIT_MS {
                return Err(PyValueError::new_err(format!(
                    "{timeout_name} must be under {SPLIT_TIMEOUT_LIMIT_MS} on a router with \
                     application='client_and_server', not {timeout_ms}: its ports forget a \
                     request after 60 seconds"
                )));
            }
            router.application_for(role).map(Some)
        }
        (_, Some(_)) => Err(PyValueError::new_err(
            "router applies only to transport='router'",
        )),
        (_, None) => Ok(None),
    }
}

#[cfg(test)]
#[path = "application_tests.rs"]
mod tests;
