//! What `bacnet_inventory` costs a 200-object controller (#2785), counted
//! on the wire and set beside the whole-device scan the CLI's `bacnet
//! inventory` runs on the same device: the numbers the tool's description,
//! the server's instructions and `docs/mcp.md` quote.

use std::time::Duration;

use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_ops::InventoryOptions;
use bacnet_types::enums::EngineeringUnits;
use serde_json::{Value, json};

use super::inventory::{TOOL, all_pages, points};
use super::site::ok;
use super::{LAB, Lab};
use crate::Policy;

/// The lab device as a controller of 200 objects: its Device object, its
/// analog inputs 1 and 3 and analog output 1, then 56 more analog inputs,
/// 36 analog outputs, 30 analog values, 30 binary inputs, 20 binary
/// outputs, 14 binary values and 10 multi-state values.
async fn controller() -> Lab {
    let lab = Lab::start_lab(Policy::default(), Duration::ZERO, 0, false).await;
    let units = EngineeringUnits::DEGREES_CELSIUS;
    for n in 100..156 {
        let ai = AnalogInputObject::new(n, format!("Zone {n} temp"), units).unwrap();
        lab.add_object(Box::new(ai)).await;
    }
    for n in 100..136 {
        let ao = AnalogOutputObject::new(n, format!("Valve {n}"), units).unwrap();
        lab.add_object(Box::new(ao)).await;
    }
    for n in 100..130 {
        let av = AnalogValueObject::new(n, format!("Setpoint {n}"), units).unwrap();
        lab.add_object(Box::new(av)).await;
    }
    for n in 100..130 {
        let bi = BinaryInputObject::new(n, format!("Status {n}")).unwrap();
        lab.add_object(Box::new(bi)).await;
    }
    for n in 100..120 {
        let bo = BinaryOutputObject::new(n, format!("Start {n}")).unwrap();
        lab.add_object(Box::new(bo)).await;
    }
    for n in 100..114 {
        let bv = BinaryValueObject::new(n, format!("Enable {n}")).unwrap();
        lab.add_object(Box::new(bv)).await;
    }
    for n in 100..110 {
        let msv = MultiStateValueObject::new(n, format!("Mode {n}"), 3).unwrap();
        lab.add_object(Box::new(msv)).await;
    }
    lab
}

/// The requests every page of `args` took together, and how many pages.
async fn paged(lab: &mut Lab, args: Value) -> (usize, usize) {
    let pages = all_pages(lab, &args).await;
    let read: usize = pages.iter().map(|(page, _)| points(page).len()).sum();
    assert_eq!(read, 200, "{args}");
    let sent = pages.iter().map(|(_, sent)| sent).sum();
    (sent, pages.len())
}

/// The requests the whole-device scan takes for `options` on the lab's
/// client: the summary, the Object_List and its batches.
async fn scanned(lab: &Lab, options: &InventoryOptions) -> usize {
    let client = lab.mcp.ctx().client.clone().unwrap();
    let device = client.device(LAB).await.unwrap();
    let before = lab.sends().len();
    let inventory = device.inventory(options).await.unwrap();
    assert_eq!(inventory.points.len(), 200);
    lab.sends().len() - before
}

/// With ReadPropertyMultiple the pages cost about what the whole scan
/// does: the list and the batches, without the scan's summary, and a few
/// more where a page ends inside what would have been one batch. Before
/// #2785 every option cost 201 (the list and one request an object). The
/// tool's description and the server's instructions quote these counts.
#[tokio::test(start_paused = true)]
async fn a_controller_costs_about_the_scans_requests() {
    let plain = InventoryOptions::new();
    let values = plain.clone().with_values(true);
    let all = values.clone().with_settings(true).with_details(true);
    // (requests, pages) through the tool; requests by the whole scan.
    for (args, options, expected, scan_expected) in [
        (json!({ "device": LAB }), plain, (28, 9), 26),
        (
            json!({ "device": LAB, "values": true }),
            values,
            (58, 12),
            56,
        ),
        (
            json!({ "device": LAB, "values": true, "settings": true, "details": true }),
            all,
            (104, 12),
            102,
        ),
    ] {
        let mut lab = controller().await;
        lab.agent.initialize().await;
        let paid = paged(&mut lab, args.clone()).await;
        let scan = scanned(&lab, &options).await;
        assert_eq!((paid, scan), (expected, scan_expected), "{args}");
        lab.stop().await;
    }
    let quoted = "a 200-object controller takes about 28 requests (58 with values, 104 with \
                  every option)";
    let description = crate::tools::description(TOOL);
    let description: String = description.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(description.contains(quoted), "{description}");
    let instructions = crate::server::INSTRUCTIONS.split_whitespace();
    let instructions = instructions.collect::<Vec<_>>().join(" ");
    assert!(
        instructions.contains("about 28 requests for a 200-object controller (58 with values)"),
        "{instructions}"
    );
}

/// The 223 export of the same controller: its 199 points (the Device
/// object isn't one) in pages, each page the device's name and its
/// batches, the first the Object_List too.
#[tokio::test(start_paused = true)]
async fn the_export_of_a_controller_costs_its_batches() {
    let mut lab = controller().await;
    lab.agent.initialize().await;
    let (mut cursor, mut sent, mut pages, mut read) = (None::<String>, 0, 0, 0);
    loop {
        let mut args = json!({ "device": LAB });
        if let Some(cursor) = &cursor {
            args["cursor"] = json!(cursor);
        }
        let before = lab.sends().len();
        let result = lab.agent.call("bacnet_export_223", args).await;
        sent += lab.sends().len() - before;
        pages += 1;
        let body = ok(&result);
        read += usize::try_from(body["read"].as_u64().unwrap()).unwrap();
        cursor = body["next_cursor"].as_str().map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(read, 199);
    // The list, six names and 29 batches; before #2785, a request a point.
    assert_eq!((sent, pages), (1 + 6 + 29, 6));
    let description = crate::tools::description("bacnet_export_223").split_whitespace();
    let description = description.collect::<Vec<_>>().join(" ");
    let quoted = "a 200-object controller takes about 36 requests";
    assert!(description.contains(quoted), "{description}");
    lab.stop().await;
}

/// Without ReadPropertyMultiple both routes cost a ReadProperty a
/// property, as before #2785: the pages the list, the refused request once
/// and the 727 properties (2 for the Device object, 3 for each analog
/// input and output, 4 for each analog value, binary input and output, 5
/// for each binary and multi-state value); the scan, the summary's 13, the
/// list and the same properties but the Device object's name, with the
/// refusal already known.
#[tokio::test(start_paused = true)]
async fn a_controller_without_rpm_costs_a_request_a_property() {
    let mut lab = controller().await;
    lab.refuse_rpm();
    lab.agent.initialize().await;
    let (sent, _) = paged(&mut lab, json!({ "device": LAB })).await;
    let properties = 2 + 3 * (58 + 37) + 4 * (30 + 30 + 20) + 5 * (14 + 10);
    assert_eq!(properties, 727);
    assert_eq!(sent, 1 + 1 + properties);
    let scan = scanned(&lab, &InventoryOptions::new()).await;
    assert_eq!(scan, 13 + 1 + properties - 1);
    lab.stop().await;
}

/// The docs quote the cost as measured above: the `bacnet_inventory` rows
/// of the tools table and the website's guide give the batched count, and
/// neither still says one request an object. They are read from the
/// repository; a copy of the crate without them skips this.
#[test]
fn the_docs_rows_quote_the_measured_cost() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let row = |path: &str| -> Option<String> {
        let text = std::fs::read_to_string(root.join(path)).ok()?;
        let row = text
            .lines()
            .find(|line| line.starts_with("| `bacnet_inventory` |"))?;
        Some(row.to_string())
    };
    let (Some(table), Some(guide)) = (
        row("docs/mcp.md"),
        row("website/src/content/docs/guides/ai-agents.md"),
    ) else {
        return;
    };
    assert!(
        table.contains("about 28 requests for a 200-object controller (58 with values)"),
        "{table}"
    );
    for row in [&table, &guide] {
        let lower = row.to_lowercase();
        assert!(!lower.contains("one request an object"), "{row}");
        assert!(!lower.contains("about 200"), "{row}");
    }
}
