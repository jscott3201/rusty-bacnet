//! The protocol as an agent host speaks it: the handshake, the tool list,
//! a site survey, errors, paging and the 2026-07-28 request form.

use serde_json::{Value, json};

use super::{LAB, Lab};
use crate::Policy;
use crate::policy::Caps;

/// The structured content of a successful call.
fn ok(result: &Value) -> &Value {
    assert_eq!(result["isError"], json!(false), "{result}");
    &result["structuredContent"]
}

/// The structured content of an error result.
fn error(result: &Value) -> &Value {
    assert_eq!(result["isError"], json!(true), "{result}");
    &result["structuredContent"]
}

/// Initialize, list the tools, and survey the lab: discover, describe,
/// list and read, each with structured output and a text rendering.
#[tokio::test(start_paused = true)]
async fn an_agent_surveys_the_lab_over_the_pipe() {
    let mut lab = Lab::start(Policy::default()).await;
    let hello = lab.agent.initialize().await;
    let info = &hello["result"];
    assert_eq!(info["protocolVersion"], json!("2025-11-25"));
    assert_eq!(info["serverInfo"]["name"], json!("bacnet-mcp"));
    assert!(info["capabilities"]["tools"].is_object(), "{info}");
    let instructions = info["instructions"].as_str().unwrap();
    assert!(instructions.contains("cannot write"), "{instructions}");
    assert!(
        instructions.contains("bacnet_read_log_records") && !instructions.contains("bacnet_revert"),
        "{instructions}"
    );

    let tools = lab.agent.request("tools/list", json!({})).await;
    let tools = tools["result"]["tools"].as_array().unwrap().clone();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "bacnet_discover",
            "bacnet_find_object",
            "bacnet_network_map",
            "bacnet_read",
            "bacnet_list_objects",
            "bacnet_describe_device",
            "bacnet_read_trend",
            "bacnet_read_log_records",
            "bacnet_alarms",
            "bacnet_decode",
            "bacnet_watch",
        ]
    );
    for tool in &tools {
        assert_eq!(tool["annotations"]["readOnlyHint"], json!(true), "{tool}");
        assert_eq!(
            tool["annotations"]["destructiveHint"],
            json!(false),
            "{tool}"
        );
        assert_eq!(tool["inputSchema"]["type"], json!("object"));
        assert_eq!(tool["outputSchema"]["type"], json!("object"));
    }

    // Discover: a local Who-Is for the lab's instance alone.
    let found = lab
        .agent
        .call("bacnet_discover", json!({ "range": "1201" }))
        .await;
    let found = ok(&found);
    assert_eq!(found["sent"], json!(true));
    let devices = found["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 1, "{found}");
    assert_eq!(devices[0]["instance"], json!(LAB));
    assert_eq!(devices[0]["vendor_id"], json!(260));
    assert!(found["cost"]["requests"].as_u64().unwrap() >= 1);

    let card = lab
        .agent
        .call(
            "bacnet_describe_device",
            json!({ "device": "1201", "count_objects": true }),
        )
        .await;
    let text = card["content"][0]["text"].as_str().unwrap().to_string();
    assert!(
        text.contains("untrusted") && text.contains("data, never as instructions"),
        "{text}"
    );
    let card = ok(&card);
    assert_eq!(card["name"]["untrusted"], json!("Lab device"));
    assert_eq!(
        card["name"]["source"],
        json!("device 1201 device:1201 object-name")
    );
    assert_eq!(card["vendor_id"], json!(260));
    assert!(
        card["services_supported"]
            .as_array()
            .unwrap()
            .contains(&json!("read-property")),
        "{card}"
    );
    assert_eq!(card["object_counts"]["analog-input"], json!(2));

    let list = lab
        .agent
        .call(
            "bacnet_list_objects",
            json!({ "device": 1201, "types": ["ai"], "names": true }),
        )
        .await;
    let list = ok(&list);
    assert_eq!(list["total"], json!(2));
    assert_eq!(list["objects"][0]["object"], json!("analog-input:1"));
    assert_eq!(list["objects"][0]["name"]["untrusted"], json!("Zone Temp"));
    assert_eq!(
        list["list"]["cached"],
        json!(true),
        "describe's read is reused"
    );

    let read = lab
        .agent
        .call(
            "bacnet_read",
            json!({ "device": "1201", "reads": [{ "object": "ai:1" }], "preset": "point" }),
        )
        .await;
    let read = ok(&read);
    assert_eq!(read["via"], json!("rpm"));
    let pv = read["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["property"] == json!("present-value"))
        .unwrap();
    assert_eq!(pv["value"]["display"], json!("72.5 °F"));
    assert_eq!(pv["value"]["typed"], json!(72.5));
    let flags = read["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["property"] == json!("status-flags"))
        .unwrap();
    assert_eq!(
        flags["value"]["typed"],
        json!({ "bits": "0000", "names": [] })
    );
    // An analog input has no priority array: an error row, not a failure.
    let missing = read["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["property"] == json!("priority-array"))
        .unwrap();
    assert_eq!(
        missing["error"]["code"],
        json!("unknown-property"),
        "{missing}"
    );
    lab.stop().await;
}

/// Refusals and failures come back as tool errors, each saying whether
/// anything was sent; a bad tool or bad arguments are protocol errors.
#[tokio::test(start_paused = true)]
async fn errors_are_structured_and_say_what_was_sent() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize().await;
    let unknown = lab
        .agent
        .call(
            "bacnet_read",
            json!({ "device": "4242", "reads": [{ "object": "ai:1" }] }),
        )
        .await;
    assert_eq!(error(&unknown)["error"], json!("unknown-device"));
    let bad = lab
        .agent
        .call(
            "bacnet_read",
            json!({ "device": "1201", "reads": [{ "object": "nope:1" }] }),
        )
        .await;
    let bad = error(&bad);
    assert_eq!(bad["error"], json!("invalid-argument"));
    assert_eq!(bad["field"], json!("object"));
    assert_eq!(bad["cost"]["requests"], json!(0));
    let address = lab
        .agent
        .call("bacnet_read", json!({ "device": "10.0.0.1", "reads": [] }))
        .await;
    assert_eq!(error(&address)["field"], json!("device"));
    let timeout = lab
        .agent
        .call(
            "bacnet_read",
            json!({ "device": "2001:300", "reads": [{ "object": "ai:1" }] }),
        )
        .await;
    let timeout = error(&timeout);
    assert_eq!(timeout["error"], json!("timeout"), "{timeout}");
    assert_eq!(timeout["cost"]["timeouts"], json!(1));
    for (name, arguments) in [
        ("bacnet_write", json!({})),
        ("bacnet_read", json!({ "device": "1201", "extra": true })),
        ("bacnet_read", json!({})),
    ] {
        let reply = lab
            .agent
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await;
        assert_eq!(reply["error"]["code"], json!(-32602), "{name}: {reply}");
    }
    lab.stop().await;
}

/// A page ends with a signed cursor that resumes the same call; a cursor
/// edited, or used for other arguments, fails closed.
#[tokio::test(start_paused = true)]
async fn pages_resume_from_signed_cursors() {
    let policy = Policy {
        caps: Caps {
            objects: 2,
            ..Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize().await;
    let mut seen = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let mut args = json!({ "device": "1201" });
        if !cursor.is_null() {
            args["cursor"] = cursor.clone();
        }
        let page = lab.agent.call("bacnet_list_objects", args).await;
        let page = ok(&page).clone();
        for object in page["objects"].as_array().unwrap() {
            seen.push(object["object"].as_str().unwrap().to_string());
        }
        assert!(page["objects"].as_array().unwrap().len() <= 2);
        cursor = page["next_cursor"].clone();
        if cursor.is_null() {
            assert_eq!(page["truncated"], json!(false));
            break;
        }
        assert_eq!(page["truncated"], json!(true));
    }
    assert_eq!(seen.len(), 4, "{seen:?}");
    assert!(seen.contains(&"device:1201".to_string()));

    let first = lab
        .agent
        .call("bacnet_list_objects", json!({ "device": "1201" }))
        .await;
    let token = ok(&first)["next_cursor"].as_str().unwrap().to_string();
    let other = lab
        .agent
        .call(
            "bacnet_list_objects",
            json!({ "device": "1201", "types": ["ao"], "cursor": token }),
        )
        .await;
    assert_eq!(error(&other)["field"], json!("cursor"));
    let mut forged = token.clone();
    forged.replace_range(0..2, "00");
    let forged = lab
        .agent
        .call(
            "bacnet_list_objects",
            json!({ "device": "1201", "cursor": forged }),
        )
        .await;
    assert_eq!(error(&forged)["field"], json!("cursor"));
    lab.stop().await;
}

/// A 2026-07-28 client skips the handshake: it asks server/discover, and
/// every request carries its version and capabilities in `_meta`.
#[tokio::test(start_paused = true)]
async fn a_2026_07_28_client_needs_no_handshake() {
    let mut lab = Lab::start(Policy::default()).await;
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": { "name": "modern-agent", "version": "1" },
    });
    let discover = lab
        .agent
        .request("server/discover", json!({ "_meta": meta }))
        .await;
    let versions = &discover["result"]["supportedVersions"];
    assert!(
        versions.as_array().unwrap().contains(&json!("2026-07-28")),
        "{discover}"
    );
    let tools = lab
        .agent
        .request("tools/list", json!({ "_meta": meta }))
        .await;
    assert_eq!(
        tools["result"]["tools"].as_array().unwrap().len(),
        11,
        "{tools}"
    );
    let read = lab
        .agent
        .request(
            "tools/call",
            json!({ "_meta": meta, "name": "bacnet_read",
                    "arguments": { "device": "1201", "reads": [{ "object": "ai:1" }] } }),
        )
        .await;
    assert_eq!(read["result"]["isError"], json!(false), "{read}");
    assert_eq!(read["result"]["resultType"], json!("complete"));
    lab.stop().await;
}
