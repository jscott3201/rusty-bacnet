//! Watch rows as an operator reads them (#1891): object names from the
//! first read, routed devices as `NET:INSTANCE`, whole object labels at
//! 80 columns and a trend that fills a wide terminal. The frames come from
//! synthetic worker reports; the name reads run against the lab device.

use std::time::Duration;

use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;

use super::site::{ClientLimits, Quirks, Server, Site};
use super::*;
use crate::tui::app::watch::{Screen, HISTORY};
use crate::tui::watched::{RowId, Sample, Source, WatchEvent};

/// Type `line` on the command bar and run it.
fn command(app: &mut App, line: &str) -> Vec<Command> {
    press(app, KeyCode::Char(':'));
    type_text(app, line);
    press(app, KeyCode::Enter)
}

fn watch(app: &mut App, event: WatchEvent) {
    worker(app, WorkerEvent::Watch(event));
}

fn sample(app: &App, value: &str, number: Option<f64>) -> Sample {
    Sample {
        value: Ok(value.into()),
        number,
        flags: None,
        at: app.now,
    }
}

/// Four rows: a routed analog input with a long name and a long history,
/// a named binary output showing its state text, a named multi-state
/// output, and an analog value whose device gave no name.
fn four_rows() -> App {
    let mut app = connected_app();
    let mut routed = row(2001, "10.0.0.9:47808", 1, 0);
    routed.network = Some(2);
    worker(&mut app, WorkerEvent::Discovered(routed));
    worker(
        &mut app,
        WorkerEvent::Discovered(row(1001, "10.0.0.2:47808", 1, 0)),
    );
    worker(
        &mut app,
        WorkerEvent::Discovered(row(1002, "10.0.0.3:47808", 1, 0)),
    );
    for line in [
        "watch 2:2001 ai:1",
        "watch 1001 bo:1",
        "watch 1001 mso:1",
        "watch 1002 av:7",
    ] {
        command(&mut app, line);
    }
    assert_eq!(app.watch.rows().len(), 4);
    let details: [(RowId, Option<&str>, Option<&str>); 4] = [
        (
            1,
            Some("Discharge Air Temperature North Wing"),
            Some("degrees-fahrenheit"),
        ),
        (2, Some("Supply Fan"), None),
        (3, Some("Fan Speed"), None),
        (4, None, Some("percent")),
    ];
    for (row, name, units) in details {
        watch(
            &mut app,
            WatchEvent::Details {
                row,
                name: name.map(Into::into),
                units: units.map(Into::into),
            },
        );
    }
    watch(
        &mut app,
        WatchEvent::Source {
            row: 1,
            source: Source::Polled("routed device".into()),
        },
    );
    for row in 2..=4 {
        watch(
            &mut app,
            WatchEvent::Source {
                row,
                source: Source::Cov { lifetime: 300 },
            },
        );
    }
    let history: Vec<(RowId, Sample)> = (0..HISTORY)
        .map(|n| {
            let value = 70.0 + (n % 16) as f64 * 0.5;
            (1, sample(&app, &format!("{value} °F"), Some(value)))
        })
        .collect();
    watch(&mut app, WatchEvent::Samples(history));
    let rest = vec![
        (2, sample(&app, "active (Running)", Some(1.0))),
        (3, sample(&app, "2 (Medium)", Some(2.0))),
        (4, sample(&app, "41.25 %", Some(41.25))),
    ];
    watch(&mut app, WatchEvent::Samples(rest));
    app.screen = Screen::Watch;
    app
}

/// At 80, 120 and 200 columns every row keeps its whole device and object
/// label; the name takes the room left; at 200 the trend fills its column.
#[test]
fn watch_rows_keep_what_identifies_them_at_every_width() {
    let mut app = four_rows();
    for (w, h) in [(80, 24), (120, 40), (200, 50)] {
        let text = screen(&mut app, w, h);
        let lines: Vec<&str> = text.lines().collect();
        for (at, (device, object)) in [
            ("2:2001", "analog-input 1"),
            ("1001", "binary-output 1"),
            ("1001", "multi-state-output 1"),
            ("1002", "analog-value 7"),
        ]
        .into_iter()
        .enumerate()
        {
            let line = lines[3 + at];
            assert!(
                line.contains(&format!("│{device} ")) && line.contains(&format!("{object} ")),
                "{w}x{h}: {line}"
            );
        }
        assert!(text.contains("active (Running)"), "{text}");
        // The line under the table leads with the source, whole.
        let detail = lines[usize::from(h) - 3];
        assert!(
            detail.starts_with("\"│ polled: routed device · analog-input 1, Discharge"),
            "{w}x{h}: {detail}"
        );
        // The routed row's trend runs to the border.
        let routed = lines[3].trim_end_matches(['"', '│']);
        assert!(
            routed.ends_with(['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█']),
            "{w}x{h}: {routed}"
        );
        assert_snapshot!(format!("watch_rows_{w}x{h}"), text);
    }
    // The history fills the trend of a terminal 600 columns wide too.
    let text = screen(&mut app, 600, 24);
    let routed = text.lines().nth(3).unwrap().trim_end_matches(['"', '│']);
    assert!(
        routed.ends_with(['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█']),
        "{routed}"
    );
}

/// `2001` and `2:2001` are one row: the device table gives the network.
#[test]
fn a_routed_device_is_one_row_however_it_is_typed() {
    let mut app = four_rows();
    command(&mut app, "watch 2001 ai:1");
    assert_eq!(app.watch.rows().len(), 4);
    let mut app = connected_app();
    let mut routed = row(2001, "10.0.0.9:47808", 1, 0);
    routed.network = Some(2);
    worker(&mut app, WorkerEvent::Discovered(routed));
    command(&mut app, "watch 2001 ai:1");
    assert_eq!(app.watch.rows()[0].point.device_label(), "2:2001");
}

/// A typed network the device table disagrees with is named in the
/// notice, and the table's network is kept; a matching or omitted one
/// says nothing about it (#1917).
#[test]
fn a_network_the_table_disagrees_with_is_named() {
    let mut app = connected_app();
    worker(
        &mut app,
        WorkerEvent::Discovered(routed_row(2001, (2, &[5]), "10.0.1.1:47808", 24, 0)),
    );
    worker(
        &mut app,
        WorkerEvent::Discovered(row(100, "10.0.0.9:47808", 1, 0)),
    );
    let flash = |app: &App| {
        app.flash
            .as_ref()
            .map(|f| f.text.clone())
            .unwrap_or_default()
    };
    command(&mut app, "watch 3:2001 ai:1");
    assert!(
        flash(&app).starts_with("2001 is on network 2, not 3. Watching"),
        "{}",
        flash(&app)
    );
    assert_eq!(app.watch.rows()[0].point.device_label(), "2:2001");
    // The same point again: the note comes with the refusal.
    command(&mut app, "watch 4:2001 ai:1");
    assert_eq!(flash(&app), "2001 is on network 2, not 4. Already watched.");
    // A local device typed with a network.
    command(&mut app, "watch 5:100 ai:1");
    assert!(
        flash(&app).starts_with("100 is on this network, not 5. Watching"),
        "{}",
        flash(&app)
    );
    assert_eq!(app.watch.rows()[1].point.device_label(), "100");
    // Matching, omitted, and routed-station forms add nothing.
    for line in [
        "watch 2:2001 ai:2",
        "watch 2001 ai:3",
        "watch 2:0x05 ai:4",
        "watch 100 ai:5",
    ] {
        command(&mut app, line);
        assert!(
            flash(&app).starts_with("Watching"),
            "{line}: {}",
            flash(&app)
        );
    }
    assert_eq!(app.watch.rows().len(), 6);
    // A device the table doesn't have keeps the network typed.
    command(&mut app, "watch 7:4000 ai:1");
    assert!(flash(&app).starts_with("Watching"), "{}", flash(&app));
    assert_eq!(app.watch.rows()[6].point.device_label(), "7:4000");
}

/// The lab device with `quirks`, discovered.
async fn lab(quirks: Quirks) -> Site {
    let server = Server {
        quirks,
        ..Server::lab(100, 100)
    };
    let mut site = Site::start_with(vec![server], ClientLimits::default()).await;
    site.who_is("", 1).await;
    site
}

/// Watch AI:1 of the lab device and run until its value is in.
async fn watch_ai1(site: &mut Site) {
    site.press(KeyCode::Char(':')).await;
    type_text(&mut site.app, "watch 100 ai:1");
    site.press(KeyCode::Enter).await;
    site.pump_until(|app| {
        app.watch
            .rows()
            .iter()
            .any(|row| !matches!(row.source, Source::Starting) && row.value.is_some())
    })
    .await;
}

fn reads(site: &Site) -> Vec<u8> {
    site.hub
        .services_to(&[10, 0, 0, 100, 0xBA, 0xC0])
        .into_iter()
        .filter(|service| [12, 14].contains(service))
        .collect()
}

/// The name comes with the first read: one ReadPropertyMultiple.
#[tokio::test(start_paused = true)]
async fn the_name_comes_with_the_first_read() {
    let mut site = lab(Quirks::default()).await;
    watch_ai1(&mut site).await;
    let row = &site.app.watch.rows()[0];
    assert_eq!(row.name.as_deref(), Some("Zone Temp"));
    assert_eq!(row.units.as_deref(), Some("degrees-fahrenheit"));
    assert_eq!(reads(&site), [14]);
    site.stop().await;
}

/// A device that fails the whole first read for Object_Name is asked
/// again without it, and the row starts nameless.
#[tokio::test(start_paused = true)]
async fn a_device_that_fails_the_read_for_the_name_still_gets_its_row() {
    let mut site = lab(Quirks {
        refuse_object_name: true,
        ..Quirks::default()
    })
    .await;
    watch_ai1(&mut site).await;
    site.run_for(Duration::from_millis(500)).await;
    let row = &site.app.watch.rows()[0];
    assert!(matches!(row.source, Source::Cov { .. }), "{:?}", row.source);
    assert_eq!(row.name, None);
    assert!(row.value.clone().unwrap().is_ok(), "{:?}", row.value);
    assert_eq!(row.units.as_deref(), Some("degrees-fahrenheit"));
    assert_eq!(reads(&site), [14, 14]);
    site.stop().await;
}

/// Read one property at a time, a refused Object_Name is that property's
/// error only: the value, flags and units still arrive.
#[tokio::test(start_paused = true)]
async fn a_refused_name_read_alone_leaves_the_rest_of_the_row() {
    let mut site = lab(Quirks {
        refuse_object_name: true,
        refuse_rpm: true,
        ..Quirks::default()
    })
    .await;
    watch_ai1(&mut site).await;
    site.run_for(Duration::from_millis(500)).await;
    let row = &site.app.watch.rows()[0];
    assert_eq!(row.name, None);
    assert!(row.value.clone().unwrap().is_ok(), "{:?}", row.value);
    assert_eq!(row.units.as_deref(), Some("degrees-fahrenheit"));
    // The refused RPM, then four ReadProperty: value, flags, units, name.
    assert_eq!(reads(&site), [14, 12, 12, 12, 12]);
    site.stop().await;
}

/// Watch `line` on the site and run until its row has left `Starting`.
async fn watch_line(site: &mut Site, line: &str) {
    site.press(KeyCode::Char(':')).await;
    type_text(&mut site.app, line);
    site.press(KeyCode::Enter).await;
    site.pump_until(|app| {
        app.watch
            .rows()
            .iter()
            .all(|row| !matches!(row.source, Source::Starting))
    })
    .await;
    // Samples reach the screen on the worker's next tick.
    site.run_for(Duration::from_millis(300)).await;
}

/// Watching Object_Name itself asks for it once, and the row's value is
/// the name.
#[tokio::test(start_paused = true)]
async fn watching_the_name_itself_asks_for_it_once() {
    let mut site = lab(Quirks::default()).await;
    watch_line(&mut site, "watch 100 ai:1 object-name").await;
    let row = &site.app.watch.rows()[0];
    assert_eq!(row.name.as_deref(), Some("Zone Temp"));
    let value = row.value.clone().unwrap().unwrap();
    assert!(value.contains("Zone Temp"), "{value}");
    assert_eq!(reads(&site), [14]);
    site.stop().await;
}

/// When the name is the watched property, a read that fails for it isn't
/// asked again: without the name there would be nothing to read.
#[tokio::test(start_paused = true)]
async fn a_refused_name_that_is_the_point_is_not_asked_again() {
    let mut site = lab(Quirks {
        refuse_object_name: true,
        ..Quirks::default()
    })
    .await;
    watch_line(&mut site, "watch 100 ai:1 object-name").await;
    assert_eq!(site.app.watch.rows()[0].name, None);
    assert_eq!(reads(&site), [14]);
    site.stop().await;
}

/// A whole-request error the name can't explain (OBJECT class here) is
/// not retried: one RPM.
#[tokio::test(start_paused = true)]
async fn an_object_error_is_not_retried_without_the_name() {
    let mut site = lab(Quirks {
        fail_rpm: true,
        ..Quirks::default()
    })
    .await;
    watch_line(&mut site, "watch 100 ai:1").await;
    assert_eq!(reads(&site), [14]);
    site.stop().await;
}

/// A device that doesn't answer isn't asked again without the name: one
/// RPM, sent with its retries (four times), then the first poll's first
/// send. A retry without the name would add four more before the poll.
#[tokio::test(start_paused = true)]
async fn a_silent_device_is_not_retried_without_the_name() {
    let mut site = lab(Quirks {
        silent: true,
        ..Quirks::default()
    })
    .await;
    watch_line(&mut site, "watch 100 ai:1").await;
    let source = site.app.watch.rows()[0].source.clone();
    assert!(
        matches!(&source, Source::Polled(reason) if reason.contains("no answer")),
        "{source:?}"
    );
    assert_eq!(reads(&site), [14; 5]);
    site.stop().await;
}

/// Two properties of one object at 80 columns: the Property column shows
/// so the rows differ, and the line under the table names the property.
#[test]
fn two_properties_of_one_object_stay_apart_at_80_columns() {
    let mut app = connected_app();
    worker(
        &mut app,
        WorkerEvent::Discovered(row(1001, "10.0.0.2:47808", 1, 0)),
    );
    command(&mut app, "watch 1001 ai:1");
    command(&mut app, "watch 1001 ai:1 status-flags");
    for row in [1, 2] {
        watch(
            &mut app,
            WatchEvent::Details {
                row,
                name: Some("Zone Temp".into()),
                units: None,
            },
        );
        watch(
            &mut app,
            WatchEvent::Source {
                row,
                source: Source::Cov { lifetime: 300 },
            },
        );
    }
    let samples = vec![
        (1, sample(&app, "72.5 °F", Some(72.5))),
        (2, sample(&app, "{in-alarm}", None)),
    ];
    watch(&mut app, WatchEvent::Samples(samples));
    app.screen = Screen::Watch;
    press(&mut app, KeyCode::Down);
    let text = screen(&mut app, 80, 24);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[2].contains("Property"), "{text}");
    assert!(lines[3].contains("present-value"), "{text}");
    assert!(lines[4].contains("status-flags"), "{text}");
    let detail = lines[21];
    assert!(
        detail.contains(
            "COV, lifetime 300 s, renewed before it runs out · analog-input 1 status-flags"
        ),
        "{detail}"
    );
    assert_snapshot!("watch_rows_properties_80x24", text);
}

/// The widest row at 80 columns: the largest instance, a long object type,
/// a value past its cap and all four flags. Device and Object stay whole;
/// the value is cut and Age and Source give way first.
#[test]
fn the_widest_row_keeps_its_device_and_object_at_80_columns() {
    let mut app = connected_app();
    worker(
        &mut app,
        WorkerEvent::Discovered(row(4_194_302, "10.0.0.2:47808", 1, 0)),
    );
    command(&mut app, "watch 4194302 multi-state-output:4194302");
    watch(
        &mut app,
        WatchEvent::Source {
            row: 1,
            source: Source::Cov { lifetime: 300 },
        },
    );
    let mut widest = sample(&app, "12 (Economizer Minimum Position)", Some(12.0));
    widest.flags = Some(crate::tui::point::Health {
        alarm: true,
        fault: true,
        overridden: true,
        out_of_service: true,
    });
    watch(&mut app, WatchEvent::Samples(vec![(1, widest)]));
    app.screen = Screen::Watch;
    let text = screen(&mut app, 80, 24);
    let line = text.lines().nth(3).unwrap();
    assert!(
        line.contains("│4194302 multi-state-output 4194302 "),
        "{text}"
    );
    assert!(line.contains("ALM FLT OVR OOS"), "{text}");
    let detail = text.lines().nth(21).unwrap();
    assert!(
        detail.contains("COV, lifetime 300 s, renewed before it runs out"),
        "{detail}"
    );
    assert_snapshot!("watch_rows_widest_80x24", text);
}
