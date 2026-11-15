//! The SC TLS PEM constructors reading files (#2600): the CA bundle read
//! first, paths named in errors and escaped, the size limit, and a real hub
//! connect through intermediates built from PEM files.
#![cfg(feature = "sc-tls")]

use std::path::Path;
use std::time::Duration;

use bacnet_transport::port::TransportPort;
use bacnet_transport::sc::ScTransport;
use bacnet_transport::sc_hub::{ScHub, ScHubTlsConfig};
use bacnet_transport::sc_tls::{
    MAX_PEM_FILE_BYTES, ScNodeTlsConfig, ScTlsConfigError, ScTlsInput, TlsWebSocket,
};

#[allow(dead_code)]
#[path = "support/sc_tls_site.rs"]
mod site;
use site::{Dir, Site, crlf};

#[test]
fn files_are_read_ca_first_and_an_unreadable_one_names_its_path() {
    let site = Site::new();
    let leaf = site.leaf();
    let dir = Dir::new("files");
    let ca = dir.write("site ca é.pem", &crlf(&site.root));
    let chain = dir.write("node.pem", &leaf.chain);
    let key = dir.write("node.key", &leaf.key.serialize_pem());
    ScNodeTlsConfig::from_pem_files(&ca, &chain, &key).unwrap();
    ScHubTlsConfig::from_pem_files(&ca, &chain, &key).unwrap();
    let missing = dir.0.join("missing.pem");
    let read = |error: ScTlsConfigError, input: ScTlsInput, path: &Path| match error {
        ScTlsConfigError::Read {
            input: i,
            path: p,
            source,
            ..
        } => {
            assert_eq!((i, p.as_path()), (input, path));
            source
        }
        other => panic!("expected a read error, got {other:?}"),
    };
    for (paths, input) in [
        ([&missing, &chain, &key], ScTlsInput::CaBundle),
        ([&ca, &missing, &key], ScTlsInput::CertificateChain),
        ([&ca, &chain, &missing], ScTlsInput::PrivateKey),
    ] {
        let error = ScNodeTlsConfig::from_pem_files(paths[0], paths[1], paths[2]).unwrap_err();
        assert!(error.to_string().contains("missing.pem"), "{error}");
        let source = read(error, input, &missing);
        assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        let error = ScHubTlsConfig::from_pem_files(paths[0], paths[1], paths[2]).unwrap_err();
        read(error, input, &missing);
    }
    // A directory is not a readable file, and the CA bundle is checked
    // before the identity files are opened at all.
    let error = ScNodeTlsConfig::from_pem_files(&dir.0, &missing, &missing).unwrap_err();
    read(error, ScTlsInput::CaBundle, &dir.0);
    let bad = dir.write(
        "bad.pem",
        "-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n",
    );
    let error = ScNodeTlsConfig::from_pem_files(&bad, &missing, &missing).unwrap_err();
    assert!(
        matches!(
            error,
            ScTlsConfigError::UnusableCertificate {
                input: ScTlsInput::CaBundle,
                ..
            }
        ),
        "{error:?}"
    );
}

/// A file is read up to the limit: one at it loads, one a byte over is
/// refused naming it, and an endless device ends at the limit.
#[test]
fn files_are_read_up_to_the_limit() {
    let site = Site::new();
    let leaf = site.leaf();
    let dir = Dir::new("limit");
    let chain = dir.write("node.pem", &leaf.chain);
    let key = dir.write("node.key", &leaf.key.serialize_pem());
    let mut padded = site.root.clone();
    padded.push_str(&" ".repeat(MAX_PEM_FILE_BYTES - padded.len()));
    let at_limit = dir.write("at-limit.pem", &padded);
    ScNodeTlsConfig::from_pem_files(&at_limit, &chain, &key).unwrap();
    padded.push(' ');
    let over = dir.write("over.pem", &padded);
    let too_large = |error: ScTlsConfigError, input: ScTlsInput, path: &Path| {
        assert!(
            matches!(&error, ScTlsConfigError::TooLarge { input: i, path: p, limit, .. }
                if *i == input && p == path && *limit == MAX_PEM_FILE_BYTES),
            "{error:?}"
        );
        assert!(
            error.to_string().contains("larger than 1048576 bytes"),
            "{error}"
        );
    };
    too_large(
        ScNodeTlsConfig::from_pem_files(&over, &chain, &key).unwrap_err(),
        ScTlsInput::CaBundle,
        &over,
    );
    let ca = dir.write("ca.pem", &site.root);
    too_large(
        ScHubTlsConfig::from_pem_files(&ca, &chain, &over).unwrap_err(),
        ScTlsInput::PrivateKey,
        &over,
    );
    #[cfg(unix)]
    {
        let zero = Path::new("/dev/zero");
        too_large(
            ScNodeTlsConfig::from_pem_files(&ca, zero, &key).unwrap_err(),
            ScTlsInput::CertificateChain,
            zero,
        );
    }
}

/// A file name's control, bidirectional formatting and line separator
/// characters are escaped in the message.
#[test]
fn read_errors_escape_control_characters_in_paths() {
    let dir = Dir::new("escape");
    let odd = dir.0.join("site\nca\u{1b}[2J.pem");
    let error = ScNodeTlsConfig::from_pem_files(&odd, &odd, &odd).unwrap_err();
    let shown = error.to_string();
    assert!(matches!(error, ScTlsConfigError::Read { .. }), "{error:?}");
    assert!(
        !shown.contains('\n') && !shown.contains('\u{1b}'),
        "{shown:?}"
    );
    assert!(shown.contains("site\\nca\\u{1b}[2J.pem"), "{shown}");
    let hidden = [
        '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}',
        '\u{2068}', '\u{2069}', '\u{2028}', '\u{2029}',
    ];
    let name: String = hidden.iter().flat_map(|c| ['x', *c]).collect();
    let odd = dir.0.join(format!("{name}.pem"));
    let shown = ScNodeTlsConfig::from_pem_files(&odd, &odd, &odd)
        .unwrap_err()
        .to_string();
    for c in hidden {
        assert!(!shown.contains(c), "{c:?} shown: {shown:?}");
        assert!(shown.contains(&c.escape_default().to_string()), "{shown}");
    }
    // Other text, an accented letter included, is shown as it is.
    let plain = dir.0.join("café ca.pem");
    let shown = ScNodeTlsConfig::from_pem_files(&plain, &plain, &plain)
        .unwrap_err()
        .to_string();
    assert!(shown.contains("café ca.pem"), "{shown}");
}

/// A hub and a node built from PEM files, each with an intermediate in its
/// chain, complete TLS 1.3 mutual authentication and the BACnet/SC connect.
#[tokio::test]
async fn hub_and_node_from_pem_files_connect_through_intermediates() {
    let site = Site::new();
    let dir = Dir::new("connect");
    let ca = dir.write("ca.pem", &site.root);
    let files = |name: &str| {
        let leaf = site.leaf();
        (
            dir.write(&format!("{name}.pem"), &crlf(&leaf.chain)),
            dir.write(&format!("{name}.key"), &leaf.key.serialize_pem()),
        )
    };
    let (hub_chain, hub_key) = files("hub");
    let (node_chain, node_key) = files("node");
    let hub_tls = ScHubTlsConfig::from_pem_files(&ca, &hub_chain, &hub_key).unwrap();
    let node_tls = ScNodeTlsConfig::from_pem_files(&ca, &node_chain, &node_key).unwrap();
    let mut hub = ScHub::start("127.0.0.1:0", hub_tls, [0x10; 6], [0x10; 16])
        .await
        .unwrap();
    let url = format!("wss://localhost:{}", hub.local_addr().unwrap().port());
    let connect = async {
        let ws = TlsWebSocket::connect(&url, node_tls).await.unwrap();
        assert_eq!(
            ws.peer_info().certificates().len(),
            2,
            "the hub sent its intermediate"
        );
        let mut transport = ScTransport::new(ws, [0x22; 6]).with_device_uuid([0x22; 16]);
        let _rx = transport.start().await.unwrap();
        assert!(transport.hub_peer_info().await.is_some());
        transport.stop().await.unwrap();
    };
    tokio::time::timeout(Duration::from_secs(10), connect)
        .await
        .expect("the PEM-built node connects to the PEM-built hub");
    hub.stop().await;
}
