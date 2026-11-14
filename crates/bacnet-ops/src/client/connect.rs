//! Connecting: a [`Client`] over a link named by the settings an
//! application sets, on the type-erased
//! [`AnyTransport`](bacnet_transport::any::AnyTransport), so the
//! application never names a transport type.
//!
//! Each builder forwards to the low-level client's own builder (the B/IP,
//! B/IPv6 and SC builders of `bacnet_client::client::BACnetClient`, and for
//! MS/TP its generic builder over an opened port), so the low-level
//! defaults and checks apply. A setting not offered here is one an
//! application wanting it sets on that builder, then wraps the client it
//! builds with [`Client::over`].

use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;

use bacnet_client::client::{BACnetClient, Bip6ClientBuilder, BipClientBuilder};
use bacnet_transport::bip::{BipMode, ForeignDeviceConfig};
use bacnet_transport::bip6::Bip6ForeignDeviceConfig;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;

use super::Client;
use crate::limits::RequestLimits;

/// `duration` in whole milliseconds, as the low-level builders take it.
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// The settings every connect builder takes: the APDU timeout and retries,
/// forwarded to the low-level builder in `builder`, and the request limits
/// the client keeps (`limits`).
macro_rules! common_settings {
    ($builder:ty) => {
        impl $builder {
            /// How long a confirmed request waits for its answer before
            /// it is sent again or fails (6 s by default).
            #[must_use]
            pub fn apdu_timeout(mut self, timeout: Duration) -> Self {
                self.builder = self.builder.apdu_timeout_ms(millis(timeout));
                self
            }

            /// How many times a confirmed request is sent again before it
            /// fails (3 by default).
            #[must_use]
            pub fn apdu_retries(mut self, retries: u8) -> Self {
                self.builder = self.builder.apdu_retries(retries);
                self
            }

            /// The request limits the client keeps: a global rate cap and a
            /// gate per device ([`RequestLimits`]). By default no rate cap,
            /// and still one request at a time to each device.
            #[must_use]
            pub fn limits(mut self, limits: RequestLimits) -> Self {
                self.limits = limits;
                self
            }
        }
    };
}

/// A client that wraps `low`, keeping `limits`.
fn wrap<T: TransportPort + 'static>(low: BACnetClient<T>, limits: RequestLimits) -> Client<T> {
    Client::new(Arc::new(low), Arc::new(limits))
}

impl Client {
    /// A BACnet/IP client, on every IPv4 interface at port 47808 until
    /// told otherwise.
    ///
    /// ```no_run
    /// use std::net::Ipv4Addr;
    /// use bacnet_ops::Client;
    /// # async fn example() -> Result<(), bacnet_types::error::Error> {
    /// let client = Client::bip()
    ///     .interface(Ipv4Addr::new(192, 168, 1, 50))
    ///     .broadcast_address(Ipv4Addr::new(192, 168, 1, 255))
    ///     .connect()
    ///     .await?;
    /// # let _ = client; Ok(()) }
    /// ```
    pub fn bip() -> BipConnect {
        BipConnect {
            builder: BACnetClient::bip_builder(),
            limits: RequestLimits::new(0),
        }
    }

    /// A BACnet/IPv6 client, on every IPv6 interface at port 47808 until
    /// told otherwise.
    pub fn bip6() -> Bip6Connect {
        Bip6Connect {
            builder: BACnetClient::bip6_builder(),
            limits: RequestLimits::new(0),
        }
    }
}

impl<T: TransportPort + 'static> Client<T> {
    /// A client over `client`, a low-level client already started, with no
    /// rate cap and one request at a time to each device: for an
    /// application that sets what the connect builders don't offer, or
    /// runs its own transport type. [`Client::new`] takes the limits too.
    pub fn over(client: BACnetClient<T>) -> Self {
        wrap(client, RequestLimits::new(0))
    }
}

/// The B/IP settings for [`Client::bip`].
pub struct BipConnect {
    builder: BipClientBuilder,
    limits: RequestLimits,
}

common_settings!(BipConnect);

impl BipConnect {
    /// The local IPv4 interface to bind (every interface by default).
    #[must_use]
    pub fn interface(mut self, ip: Ipv4Addr) -> Self {
        self.builder = self.builder.interface(ip);
        self
    }

    /// The UDP port (47808 by default; 0 for any free one).
    #[must_use]
    pub fn port(mut self, port: u16) -> Self {
        self.builder = self.builder.port(port);
        self
    }

    /// The directed broadcast address local broadcasts go to
    /// (255.255.255.255 by default; the subnet's, such as 192.168.1.255,
    /// on a host with several interfaces).
    #[must_use]
    pub fn broadcast_address(mut self, address: Ipv4Addr) -> Self {
        self.builder = self.builder.broadcast_address(address);
        self
    }

    /// Have the BBMD `config` names take this client on as a foreign
    /// device: it then hands the client the broadcasts of its BACnet
    /// network, and passes the client's broadcasts on there (Annex J). The client starts without waiting for
    /// the BBMD to accept; [`BACnetClient::foreign_registration`] on its
    /// [`low_level`](Client::low_level) client says how it went.
    #[must_use]
    pub fn foreign_device(mut self, config: ForeignDeviceConfig) -> Self {
        self.builder = self.builder.mode(BipMode::Foreign(config));
        self
    }

    /// Bind the socket and start the client.
    ///
    /// # Errors
    ///
    /// As the low-level builder's `build_any`: settings it refuses, and a
    /// socket that can't be bound.
    pub async fn connect(self) -> Result<Client, Error> {
        let low = Box::pin(self.builder.build_any()).await?;
        Ok(wrap(low, self.limits))
    }
}

/// The B/IPv6 settings for [`Client::bip6`].
pub struct Bip6Connect {
    builder: Bip6ClientBuilder,
    limits: RequestLimits,
}

common_settings!(Bip6Connect);

impl Bip6Connect {
    /// The local IPv6 interface to bind (every interface by default).
    #[must_use]
    pub fn interface(mut self, ip: Ipv6Addr) -> Self {
        self.builder = self.builder.interface(ip);
        self
    }

    /// The UDP port (47808 by default; 0 for any free one).
    #[must_use]
    pub fn port(mut self, port: u16) -> Self {
        self.builder = self.builder.port(port);
        self
    }

    /// Have the B/IPv6 BBMD `config` names take this client on as a
    /// foreign device (Annex U).
    #[must_use]
    pub fn foreign_device(mut self, config: Bip6ForeignDeviceConfig) -> Self {
        self.builder = self.builder.foreign_device(config);
        self
    }

    /// Bind the socket, join the multicast group and start the client.
    ///
    /// # Errors
    ///
    /// As the low-level builder's `build_any`.
    pub async fn connect(self) -> Result<Client, Error> {
        let low = Box::pin(self.builder.build_any()).await?;
        Ok(wrap(low, self.limits))
    }
}

#[cfg(feature = "sc-tls")]
mod sc {
    use bacnet_client::client::ScClientBuilder;
    use bacnet_transport::sc::ScReconnectConfig;
    use bacnet_transport::sc_tls::ScNodeTlsConfig;

    use super::*;

    impl Client {
        /// A BACnet/SC client of the hub at `hub_url`
        /// (`wss://hub.example.com/bacnet`), with `tls` as its TLS
        /// identity and trust. It needs its VMAC and device UUID too
        /// ([`ScConnect::vmac`], [`ScConnect::device_uuid`]).
        pub fn sc(hub_url: &str, tls: ScNodeTlsConfig) -> ScConnect {
            ScConnect {
                builder: BACnetClient::sc_builder().hub_url(hub_url).tls_config(tls),
                limits: RequestLimits::new(0),
            }
        }
    }

    /// The BACnet/SC settings for [`Client::sc`].
    pub struct ScConnect {
        pub(super) builder: ScClientBuilder,
        pub(super) limits: RequestLimits,
    }

    common_settings!(ScConnect);

    impl ScConnect {
        /// This node's VMAC on the hub (required).
        #[must_use]
        pub fn vmac(mut self, vmac: [u8; 6]) -> Self {
            self.builder = self.builder.vmac(vmac);
            self
        }

        /// This node's persistent device UUID (required, not all zero).
        #[must_use]
        pub fn device_uuid(mut self, uuid: [u8; 16]) -> Self {
            self.builder = self.builder.device_uuid(uuid);
            self
        }

        /// The failover hub, dialled with the same TLS settings when the
        /// primary can't be reached. Needs [`reconnect`](Self::reconnect).
        #[must_use]
        pub fn failover_hub_url(mut self, url: &str) -> Self {
            self.builder = self.builder.failover_hub_url(url);
            self
        }

        /// Redial a lost hub connection as `config` says (by default a
        /// lost connection stays down).
        #[must_use]
        pub fn reconnect(mut self, config: ScReconnectConfig) -> Self {
            self.builder = self.builder.reconnect(config);
            self
        }

        /// Dial the hub and start the client.
        ///
        /// # Errors
        ///
        /// As the low-level builder's `build_any`: a missing or reserved
        /// VMAC or UUID and other refused settings before any dial, then a
        /// hub that can't be reached or refuses the connection.
        pub async fn connect(self) -> Result<Client, Error> {
            let low = Box::pin(self.builder.build_any()).await?;
            Ok(wrap(low, self.limits))
        }
    }
}

#[cfg(feature = "sc-tls")]
pub use self::sc::ScConnect;

#[cfg(feature = "serial")]
mod mstp {
    use bacnet_client::client::ClientBuilder;
    use bacnet_transport::any::AnyTransport;
    use bacnet_transport::mstp::{MstpConfig, MstpTransport};
    use bacnet_transport::mstp_serial::{SerialConfig, TokioSerialPort};

    use super::*;

    /// The baud rate an MS/TP client joins at unless told otherwise.
    const DEFAULT_BAUD: u32 = 38_400;

    impl Client {
        /// An MS/TP client on the serial port `port` (`/dev/ttyUSB0`,
        /// `COM3`), joining the trunk as master station `mac` (0 to 127,
        /// one no other device uses) at 38,400 baud until told otherwise.
        pub fn mstp(port: &str, mac: u8) -> MstpConnect {
            MstpConnect {
                builder: BACnetClient::<AnyTransport>::generic_builder(),
                limits: RequestLimits::new(0),
                port: port.to_string(),
                config: MstpConfig {
                    this_station: mac,
                    baud_rate: DEFAULT_BAUD,
                    ..MstpConfig::default()
                },
            }
        }
    }

    /// The MS/TP settings for [`Client::mstp`].
    pub struct MstpConnect {
        pub(super) builder: ClientBuilder<AnyTransport>,
        pub(super) limits: RequestLimits,
        port: String,
        config: MstpConfig,
    }

    common_settings!(MstpConnect);

    impl MstpConnect {
        /// The trunk's baud rate: 9600, 19200, 38400, 57600, 76800 or
        /// 115200.
        #[must_use]
        pub fn baud(mut self, baud: u32) -> Self {
            self.config.baud_rate = baud;
            self
        }

        /// Check the settings, open the serial port and join the trunk.
        ///
        /// # Errors
        ///
        /// Settings the MS/TP transport refuses (a MAC above Max_Master,
        /// a baud rate too low), before the port is opened; a port that
        /// can't be opened; then as the low-level builder's `build`.
        pub async fn connect(self) -> Result<Client, Error> {
            self.config.validate()?;
            let serial = TokioSerialPort::open(&SerialConfig {
                port_name: self.port.clone(),
                baud_rate: self.config.baud_rate,
            })
            .map_err(|error| match error {
                Error::Encoding(message) => Error::Encoding(format!("{}: {message}", self.port)),
                other => other,
            })?;
            let max_apdu = self.config.max_apdu_length();
            let transport = MstpTransport::new(serial, self.config);
            let low = Box::pin(
                self.builder
                    .transport(AnyTransport::from(transport))
                    .max_apdu_length(max_apdu)
                    .build(),
            )
            .await?;
            Ok(wrap(low, self.limits))
        }
    }
}

#[cfg(feature = "serial")]
pub use self::mstp::MstpConnect;

#[cfg(test)]
#[path = "connect_tests.rs"]
mod tests;
