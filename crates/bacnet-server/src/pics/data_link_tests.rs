//! The data-link options a PICS draft lists.

use bacnet_objects::database::ObjectDatabase;

use super::*;

/// The NAT traversal data-link option (Annex J.7.5, #940) is its own line,
/// beside plain B/IP, in both renderings.
#[test]
fn the_nat_traversal_option_is_listed() {
    let db = ObjectDatabase::new();
    let pics_config = PicsConfig {
        data_link_layers: vec![DataLinkSupport::BipV4, DataLinkSupport::BipV4NatTraversal],
        ..PicsConfig::default()
    };
    let pics = generate_pics(&db, &ServerConfig::default(), &pics_config);
    let line = "BACnet/IP with NAT traversal (Annex J.7.5)";
    assert!(pics.generate_text().contains(&format!("  {line}\n")));
    assert!(pics.generate_markdown().contains(&format!("- {line}\n")));
    assert!(pics.generate_text().contains("  BACnet/IP (Annex J)\n"));
}
