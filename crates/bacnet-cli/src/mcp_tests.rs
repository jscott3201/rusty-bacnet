//! The `[mcp]` table as a server policy, and the first-use authorization.

use super::*;

/// A scripted person at the terminal.
struct Typed {
    interactive: bool,
    answer: Option<&'static str>,
    asked: Vec<String>,
}

impl Confirm for Typed {
    fn interactive(&self) -> bool {
        self.interactive
    }
    fn ask(&mut self, prompt: &str) -> Option<String> {
        self.asked.push(prompt.into());
        self.answer.map(|a| format!("{a}\n"))
    }
}

fn typed(interactive: bool, answer: Option<&'static str>) -> Typed {
    Typed {
        interactive,
        answer,
        asked: Vec::new(),
    }
}

/// Whether the stored profile's consent stands.
fn authorized(store: &Store) -> bool {
    store
        .load("site")
        .unwrap()
        .mcp
        .is_some_and(|mcp| mcp.authorized())
}

fn store_with(section: Option<McpSection>) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let profile = Profile {
        mcp: section.map(Box::new),
        ..Profile::default()
    };
    store.save("site", &profile, true).unwrap();
    (dir, store)
}

#[test]
fn the_table_becomes_the_policy() {
    let section = McpSection {
        networks: Some(vec![0, 2001]),
        devices: Some(vec!["1000-1999".into(), "5".into()]),
        slow_networks: vec![2001],
        max_rate: Some(10),
        call_budget_s: Some(45),
        ..McpSection::default()
    };
    let mapped = policy(&section, false).unwrap();
    assert_eq!(mapped.networks, Some([0, 2001].into()));
    assert_eq!(mapped.devices, Some(vec![(1000, 1999), (5, 5)]));
    assert_eq!(mapped.rates.global, 10);
    assert_eq!(mapped.rates.principal, Rates::default().principal);
    assert_eq!(mapped.caps.budget, Duration::from_secs(45));
    assert!(!mapped.passive && mapped.slow_networks.contains(&2001));
    assert_eq!(
        policy(&McpSection::default(), true).unwrap(),
        Policy {
            passive: true,
            ..Policy::default()
        }
    );
    let bad = McpSection {
        principal_rate: Some(0),
        ..McpSection::default()
    };
    assert!(policy(&bad, false).unwrap_err().starts_with("[mcp]"));
}

/// The profile name typed at a terminal records consent and the table it
/// covered; anything else, or no terminal, records nothing.
#[test]
fn authorization_needs_a_person_to_type_the_name() {
    let section = McpSection {
        networks: Some(vec![0]),
        ..McpSection::default()
    };
    let (_dir, store) = store_with(Some(section.clone()));
    for mut person in [
        typed(false, Some("site")),
        typed(true, Some("other")),
        typed(true, None),
    ] {
        assert!(authorize(&store, "site", &mut person).is_err());
        assert!(!authorized(&store));
    }
    let mut person = typed(true, Some("site"));
    authorize(&store, "site", &mut person).unwrap();
    assert!(
        person.asked[0].contains("networks: [0]"),
        "{}",
        person.asked[0]
    );
    let stored = *store.load("site").unwrap().mcp.unwrap();
    assert!(stored.authorized_at_unix.is_some());
    assert_eq!(stored.authorized.as_deref(), Some(&section));
}

/// Widening the table after authorizing clears the authorization when the
/// profile is next loaded; narrowing it doesn't.
#[test]
fn widening_reach_needs_authorizing_again() {
    let (_dir, store) = store_with(Some(McpSection {
        networks: Some(vec![0]),
        ..McpSection::default()
    }));
    authorize(&store, "site", &mut typed(true, Some("site"))).unwrap();
    let mut profile = store.load("site").unwrap();
    profile.mcp.as_mut().unwrap().devices = Some(vec!["1-10".into()]);
    store.save("site", &profile, false).unwrap();
    assert!(authorized(&store));
    profile.mcp.as_mut().unwrap().networks = None;
    store.save("site", &profile, false).unwrap();
    assert!(!authorized(&store));
    assert!(unauthorized("site").contains("bacnet mcp authorize --profile site"));
}

/// Raising a rate or a cap, or changing the transport, also needs
/// authorizing again.
#[test]
fn faster_or_another_transport_needs_authorizing_again() {
    let changes: [fn(&mut Profile); 4] = [
        |p| p.mcp.as_mut().unwrap().max_rate = Some(50),
        |p| p.mcp.as_mut().unwrap().call_budget_s = Some(120),
        |p| p.mcp.as_mut().unwrap().page_objects = Some(1000),
        |p| p.bip.as_mut().unwrap().port = 47809,
    ];
    for change in changes {
        let (_dir, store) = store_with(Some(McpSection::default()));
        authorize(&store, "site", &mut typed(true, Some("site"))).unwrap();
        assert!(authorized(&store));
        let mut profile = store.load("site").unwrap();
        change(&mut profile);
        store.save("site", &profile, false).unwrap();
        assert!(!authorized(&store), "{profile:?}");
    }
}

/// A consent time without the table and transport it covered, as a hand
/// edit might leave, authorizes nothing.
#[test]
fn a_bare_consent_time_authorizes_nothing() {
    let (_dir, store) = store_with(Some(McpSection {
        authorized_at_unix: Some(1),
        ..McpSection::default()
    }));
    assert!(!authorized(&store));
    let stored = store.load("site").unwrap().mcp.unwrap();
    assert_eq!(stored.authorized_at_unix, None, "cleared on load");
}

/// `mcp_writes`, `mcp_arm` and `[mcp.plan]` become the write policy, the
/// plan running from the authorization (#2068); authorizing says plainly
/// what a plan lets through without a prompt; and offering writes or
/// changing the plan needs authorizing again.
#[test]
fn writes_and_a_plan_become_the_write_policy() {
    use crate::tui::profile::mcp::{McpChoice, McpPlan};
    let plan = McpPlan {
        devices: vec!["1201".into()],
        properties: vec!["present-value".into()],
        priority_min: 8,
        priority_max: 16,
        max_writes: 50,
        hours: 2,
        reason: Some("commissioning".into()),
    };
    let section = McpSection {
        mcp_writes: McpChoice::Elicit,
        mcp_arm: McpChoice::Elicit,
        plan: Some(plan.clone()),
        authorized_at_unix: Some(1_000),
        ..McpSection::default()
    };
    let workspace = Workspace {
        dir: "/tmp/site".into(),
        profile: Some("site".into()),
        transport: "BIP",
        local: "0.0.0.0:47808".into(),
    };
    let writes = write_policy(&section, workspace.clone()).unwrap();
    assert_eq!(writes.mode, WriteMode::Elicit);
    assert_eq!(writes.arm, ArmMode::Elicit);
    let grant = writes.plan.unwrap();
    assert_eq!(grant.devices, [(1201, 1201)]);
    assert_eq!(grant.priorities, (8, 16));
    assert_eq!(grant.expires_unix, 1_000 + 2 * 3_600);

    let (_dir, store) = store_with(Some(McpSection {
        authorized_at_unix: None,
        ..section.clone()
    }));
    let mut person = typed(true, Some("site"));
    authorize(&store, "site", &mut person).unwrap();
    assert!(
        person.asked[0].contains("WITHOUT ANY PROMPT up to 50 writes"),
        "{}",
        person.asked[0]
    );
    // Authorizing starts the plan's write count at zero (#2159), under the
    // key `serve` computes from the stored profile.
    let site = store.workspace("site").unwrap();
    let stored = store.load("site").unwrap().mcp.unwrap();
    let served = Workspace {
        dir: site.clone(),
        ..workspace.clone()
    };
    let grant = write_policy(&stored, served).unwrap().plan.unwrap();
    assert_eq!(grant.budget_used(&site), Ok(0));
    let mut profile = store.load("site").unwrap();
    let mcp = profile.mcp.as_mut().unwrap();
    mcp.plan.as_mut().unwrap().max_writes = 51;
    store.save("site", &profile, false).unwrap();
    assert!(!authorized(&store), "a changed plan");

    let never = McpSection {
        plan: Some(plan),
        ..McpSection::default()
    };
    assert!(never.validate().is_err(), "a plan needs mcp_writes");
    let offered = McpSection {
        mcp_writes: McpChoice::Elicit,
        ..McpSection::default()
    };
    assert!(offered.raises(&McpSection::default()));
}

/// A future authorization time (a hand edit) can't lengthen a plan; the
/// summary warns when a plan reaches the life-safety priorities, and when
/// writes are listed but nothing can arm them; the audit's local address
/// keeps no credential.
#[test]
fn plans_start_no_later_than_now_and_summaries_warn() {
    use crate::tui::profile::mcp::{McpChoice, McpPlan};
    let plan = McpPlan {
        devices: vec!["1".into()],
        properties: vec!["present-value".into()],
        priority_min: 1,
        priority_max: 16,
        max_writes: 5,
        hours: 1,
        reason: None,
    };
    let future = McpSection {
        mcp_writes: McpChoice::Elicit,
        plan: Some(plan),
        authorized_at_unix: Some(u64::MAX / 2),
        ..McpSection::default()
    };
    let workspace = Workspace {
        dir: "/tmp/site".into(),
        profile: None,
        transport: "BIP",
        local: String::new(),
    };
    let grant = write_policy(&future, workspace).unwrap().plan.unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(grant.expires_unix <= now + 3_600, "{}", grant.expires_unix);
    assert!(summary("site", &future).contains("CAUTION: priority 1, 2, 5"));
    let unarmable = McpSection {
        mcp_writes: McpChoice::Elicit,
        ..McpSection::default()
    };
    assert!(summary("site", &unarmable).contains("refused (not-armed)"));
    assert_eq!(
        without_secrets("wss://user:pw@hub.example:443/sc?token=abc#x"),
        "wss://hub.example:443/sc"
    );
    assert_eq!(without_secrets("wss://hub.example"), "wss://hub.example");
}

/// The defaults an absent key means agree with the server's.
#[test]
fn the_profile_defaults_are_the_servers() {
    use crate::tui::profile::mcp::{DEFAULT_CAPS, DEFAULT_RATES};
    let (rates, caps) = (Rates::default(), Caps::default());
    assert_eq!(
        DEFAULT_RATES,
        [
            rates.global,
            rates.network,
            rates.slow,
            rates.fast,
            rates.principal,
            rates.burst
        ]
    );
    let size = |n: usize| u64::try_from(n).unwrap();
    assert_eq!(
        DEFAULT_CAPS,
        [
            size(caps.values),
            size(caps.devices),
            size(caps.objects),
            size(caps.text),
            caps.budget.as_secs()
        ]
    );
}
