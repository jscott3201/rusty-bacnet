//! The website's sample PICS, `website/src/content/docs/project/sample-pics.md`
//! (#1496).
//!
//! It is the full server's own PICS, from `BACnetServer::generate_pics`, for
//! one documented reference configuration:
//!
//! - one object of each constructible type from `bacnet_objects::catalogue`,
//!   with the catalogue's Device swapped for one carrying the reference
//!   identity below;
//! - BACnet/IP and BACnet/SC declared as its data links;
//! - router, BBMD and foreign-device registration off;
//! - fixed placeholder vendor and model names on the Device, its version
//!   fields set to the release the site describes (from
//!   `website/src/lib/site.mjs`, as for `support.json`), so the file changes
//!   only when the code or the release does. The generator takes these and
//!   the protocol version and revision from the served Device.
//!
//! The server runs on BACnet/IP on loopback just long enough to answer. The
//! generator's Markdown goes in below a banner that names the configuration
//! and says the file describes it alone; its headings move down one level
//! under the page title, between `generated:start` and `generated:end`
//! markers, which give the page the low search weight of the generated
//! reference pages (`DocsMarkdownContent.astro`). The banner is part of the
//! file, so the page's plain-text export carries it too.
//!
//! `cargo run -p bacnet-integration-tests --example support-tables` writes
//! this file beside `support.json`, and `tests/support_tables.rs` fails when
//! the committed copy differs from a fresh run.

use std::net::Ipv4Addr;
use std::path::Path;

use bacnet_objects::DeviceIdentity;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::pics::{DataLinkSupport, DeviceProfile, NetworkLayerSupport, PicsConfig};
use bacnet_server::server::BACnetServer;
use bacnet_types::enums::ObjectType;

use crate::support_tables::{FORBIDDEN_WORDS, site_release};

/// Where the generated page goes, relative to the repository root.
pub const OUTPUT: &str = "website/src/content/docs/project/sample-pics.md";

/// The reference configuration's name, as the title and banner give it.
pub const REFERENCE: &str = "catalogue reference configuration";

/// The reference Device's instance. Every catalogue object is instance 1.
const DEVICE_INSTANCE: u32 = 1;

/// A placeholder: a real device uses the vendor identifier ASHRAE assigned
/// its vendor.
const VENDOR_ID: u16 = 0;

const VENDOR_NAME: &str = "Placeholder (reference configuration)";

const MODEL_NAME: &str = "Rusty BACnet catalogue reference";

type Result<T> = std::result::Result<T, String>;

/// The PICS settings the reference configuration declares.
fn pics_config() -> PicsConfig {
    PicsConfig {
        device_profile: DeviceProfile::Custom("None claimed".into()),
        data_link_layers: vec![DataLinkSupport::BipV4, DataLinkSupport::BacnetSc],
        network_layer: NetworkLayerSupport {
            router: false,
            bbmd: false,
            foreign_device: false,
        },
        ..PicsConfig::default()
    }
}

/// The catalogue, with its Device replaced by one with the reference identity;
/// `version` fills both of its version fields.
fn database(identity: &DeviceIdentity, version: &str) -> Result<ObjectDatabase> {
    let device = DeviceObject::new(DeviceConfig {
        identity: identity.clone(),
        vendor_name: VENDOR_NAME.into(),
        model_name: MODEL_NAME.into(),
        firmware_revision: version.into(),
        application_software_version: version.into(),
        ..DeviceConfig::default()
    })
    .map_err(|error| format!("reference Device: {error}"))?;
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device))
        .map_err(|error| format!("reference Device: {error}"))?;
    for object in bacnet_objects::catalogue::representatives() {
        if object.object_identifier().object_type() == ObjectType::DEVICE {
            continue;
        }
        db.add(object).map_err(|error| error.to_string())?;
    }
    Ok(db)
}

/// The generator's Markdown for the reference configuration.
async fn pics_markdown(version: &str) -> Result<String> {
    let identity = DeviceIdentity::new(DEVICE_INSTANCE, VENDOR_ID)
        .map_err(|error| error.to_string())?
        .with_name("Catalogue reference device");
    let mut server = BACnetServer::bip_builder()
        .device(identity.clone())
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(database(&identity, version)?)
        .build()
        .await
        .map_err(|error| format!("start the reference server: {error}"))?;
    server
        .stop()
        .await
        .map_err(|error| format!("stop the reference server: {error}"))?;
    let pics = server.generate_pics(&pics_config()).await;
    Ok(pics.generate_markdown())
}

/// Moves every Markdown heading down one level, so the generator's title
/// sits under the page's.
fn demote_headings(markdown: &str) -> String {
    markdown
        .lines()
        .map(|line| {
            if line.starts_with('#') {
                format!("#{line}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The whole page, as written: frontmatter, banner, the reference
/// configuration, then the generator's output.
pub fn generate(repo: &Path) -> Result<String> {
    let site = std::fs::read_to_string(repo.join("website/src/lib/site.mjs"))
        .map_err(|error| format!("read site.mjs: {error}"))?
        .replace("\r\n", "\n");
    let release = site_release(&site)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    let pics = demote_headings(runtime.block_on(pics_markdown(&release))?.trim_end());
    let text = format!(
        r#"---
title: "Sample PICS: the {REFERENCE}"
description: "Generated PICS output for the {REFERENCE} only, one object of each constructible type on BACnet/IP and BACnet/SC: an example of the generator's output, not a claim for any device."
---

<!-- Generated by crates/bacnet-integration-tests/sample_pics.rs; run `cargo run -p bacnet-integration-tests --example support-tables`, do not edit. -->

:::caution[Generated sample for the {REFERENCE} only]
This page is the PICS generator's output for the {REFERENCE} described below, and for that configuration alone. It is not a claim for any product or device, nor a test result or a BTL listing. Don't copy it as your device's PICS: [generate one from your own configuration](/rusty-bacnet/integrate/pics/).
:::

[Evaluate and get help](/rusty-bacnet/project/support/) / Sample PICS

## The {REFERENCE}

The PICS below comes from `BACnetServer::generate_pics` on a full server built for this page and nothing else:

- **Objects:** one of each type that `bacnet_objects::catalogue::representatives()` constructs, each instance 1 with the catalogue's placeholder names and units. The catalogue's Device is replaced by Device {DEVICE_INSTANCE} with vendor identifier {VENDOR_ID}, a placeholder.
- **Data links:** BACnet/IP and BACnet/SC, as declared in its `PicsConfig`. The generator doesn't detect data links; the server that produced this page ran on BACnet/IP on loopback.
- **Network layer:** router, BBMD and foreign-device registration off.
- **Device fields:** the Device's vendor name and model name are placeholders, and its firmware and application versions name v{release}, the release this site describes. The PICS takes these, and the protocol version and revision, from that Device object.
- **Declared fields:** the device profile is left unclaimed.

The file is generated on the `dev` branch, like the [capability tables](/rusty-bacnet/project/conformance/), so it can include object types or services merged after v{release}. [Generate a PICS for your device](/rusty-bacnet/integrate/pics/) explains each section and what you fill in yourself.

<!-- generated:start by crates/bacnet-integration-tests/sample_pics.rs: the PICS generator's output -->

{pics}

<!-- generated:end -->
"#
    );
    let lower = text.to_lowercase();
    if let Some(word) = FORBIDDEN_WORDS.iter().find(|word| lower.contains(*word)) {
        return Err(format!("the sample PICS must not say {word:?}"));
    }
    Ok(text)
}
