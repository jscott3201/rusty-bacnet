//! MS/TP profile settings (#969): the `[mstp]` table. Joining still needs
//! the T2 confirmation at each connection; the profile only remembers the
//! settings.

use serde::{Deserialize, Serialize};

use crate::mstp::{DEFAULT_BAUD, MAX_MASTER_MAC, MstpArgs};

/// MS/TP settings. `port` and `mac` have no default: both are a choice
/// about this trunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Mstp {
    /// The serial port, as `bacnet mstp-ports` lists it.
    pub(crate) port: String,
    /// This station's MAC, 0 to 127.
    pub(crate) mac: u8,
    /// One of the Clause 9.2.3 baud rates.
    #[serde(default = "default_baud")]
    pub(crate) baud: u32,
    /// Max_Master, at least `mac`.
    #[serde(default = "default_max_master")]
    pub(crate) max_master: u8,
    /// Max_Info_Frames, 1 to 255.
    #[serde(default = "default_max_info_frames")]
    pub(crate) max_info_frames: u8,
    /// APDU timeout.
    #[serde(default = "default_timeout_ms")]
    pub(crate) timeout_ms: u64,
    /// Minimum spacing between confirmed requests (0: none).
    #[serde(default)]
    pub(crate) min_interval_ms: u64,
}

fn default_baud() -> u32 {
    DEFAULT_BAUD
}

fn default_max_master() -> u8 {
    MAX_MASTER_MAC
}

fn default_max_info_frames() -> u8 {
    1
}

fn default_timeout_ms() -> u64 {
    6000
}

impl Default for Mstp {
    /// A new profile's starting point; it validates only once a port is
    /// filled in.
    fn default() -> Self {
        Self {
            port: String::new(),
            mac: 0,
            baud: default_baud(),
            max_master: default_max_master(),
            max_info_frames: default_max_info_frames(),
            timeout_ms: default_timeout_ms(),
            min_interval_ms: 0,
        }
    }
}

impl Mstp {
    /// The settings in the CLI's form.
    pub(crate) fn args(&self) -> MstpArgs {
        MstpArgs {
            port: self.port.clone(),
            baud: self.baud,
            mac: self.mac,
            max_master: self.max_master,
            max_info_frames: self.max_info_frames,
        }
    }

    /// The flags' checks, and request pacing's limit.
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.args().validate()?;
        if self.min_interval_ms > bacnet_client::client::MAX_MIN_REQUEST_INTERVAL_MS {
            return Err("min_interval_ms must be at most 3600000".into());
        }
        Ok(())
    }
}
