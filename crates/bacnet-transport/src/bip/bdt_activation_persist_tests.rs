//! A BDT activated through the Network Port object is saved to the
//! persisted-BDT file, which a restarted transport loads; a failed save is
//! counted and leaves the activation in place (#1826).

use super::*;
use bacnet_types::bip_port::BbmdChanges;
use bacnet_types::constructed::{BACnetBDTEntry, BACnetHostNPort};
use bytes::BytesMut;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// A directory of its own under the system's temporary directory, removed
/// with everything in it when dropped.
pub(super) struct TempDir(PathBuf);

impl TempDir {
    pub(super) fn new(label: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "rusty-bacnet-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub(super) fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn row(ip: [u8; 4], port: u16) -> BACnetBDTEntry {
    BACnetBDTEntry {
        bbmd_address: BACnetHostNPort::from_socket_addr(SocketAddrV4::new(ip.into(), port).into()),
        broadcast_mask: Some([0xFF; 4]),
    }
}

/// A started BBMD on loopback with an empty configured BDT and `path` as
/// its persisted-BDT file.
async fn bbmd_at(path: &Path) -> BipTransport {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.enable_bbmd(Vec::new());
    transport.set_bdt_persist_path(path.to_path_buf());
    let _rx = transport.start().await.unwrap();
    transport
}

pub(super) fn tables(transport: &BipTransport) -> Arc<dyn BbmdTables> {
    match transport.bip_port().unwrap().mode {
        BipPortMode::Bbmd {
            tables: Some(tables),
        } => tables,
        other => panic!("{other:?}"),
    }
}

/// Wait until `count` saves of `transport`'s file have finished: they run
/// on blocking threads once the activation returns.
pub(super) async fn saved(transport: &BipTransport, count: u64) {
    let file = Arc::clone(transport.bdt_file.as_ref().unwrap());
    tokio::time::timeout(Duration::from_secs(10), async {
        while file.finished() < count {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the save finished");
}

/// The temporary files left in `dir`.
fn temporaries(dir: &TempDir) -> Vec<PathBuf> {
    fs::read_dir(&dir.0)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "tmp"))
        .collect()
}

fn bdt_changes(table: Vec<BACnetBDTEntry>) -> BbmdChanges {
    let mut changes = BbmdChanges::default();
    changes.broadcast_distribution_table = Some(table);
    changes
}

#[tokio::test]
async fn an_activated_bdt_is_saved_and_a_restart_loads_it() {
    let dir = TempDir::new("bdt-activation");
    let path = dir.file("bdt.bin");
    let mut transport = bbmd_at(&path).await;
    assert!(
        !path.exists(),
        "nothing is written until a BDT is activated"
    );
    let own = transport.bip_port().unwrap().endpoint;
    let table = vec![
        row([10, 0, 0, 2], 0xBAC0),
        row(own.ip().octets(), own.port()),
    ];

    // A change that leaves the BDT alone doesn't touch the file.
    let mut accept = BbmdChanges::default();
    accept.accept_foreign_device_registrations = Some(true);
    tables(&transport).activate(&accept).unwrap();
    assert!(!path.exists());

    tables(&transport)
        .activate(&bdt_changes(table.clone()))
        .unwrap();
    assert_eq!(tables(&transport).broadcast_distribution_table(), table);
    saved(&transport, 1).await;
    let mut expected = BytesMut::new();
    let rows = [
        bbmd::BdtEntry {
            ip: [10, 0, 0, 2],
            port: 0xBAC0,
            broadcast_mask: [0xFF; 4],
        },
        bbmd::BdtEntry {
            ip: own.ip().octets(),
            port: own.port(),
            broadcast_mask: [0xFF; 4],
        },
    ];
    bbmd::encode_bdt_entries(&rows, &mut expected);
    // Rows only: no activation has set the NAT rows (#940).
    assert_eq!(fs::read(&path).unwrap(), expected.to_vec());
    assert!(
        temporaries(&dir).is_empty(),
        "the temporary file is renamed"
    );
    assert_eq!(transport.bdt_persist_failures(), 0);
    transport.stop().await.unwrap();

    // A new transport with the same file starts from the activated BDT, not
    // its configured (empty) one.
    let mut restarted = BipTransport::new(Ipv4Addr::LOCALHOST, own.port(), Ipv4Addr::BROADCAST);
    restarted.enable_bbmd(Vec::new());
    restarted.set_bdt_persist_path(path.clone());
    let _rx = restarted.start().await.unwrap();
    assert_eq!(tables(&restarted).broadcast_distribution_table(), table);
    restarted.stop().await.unwrap();
}

#[tokio::test]
async fn a_failed_save_is_counted_and_keeps_the_activation() {
    let dir = TempDir::new("bdt-activation-failure");
    // The file's directory doesn't exist, so neither the temporary file nor
    // the rename can be made.
    let path = dir.file("missing").join("bdt.bin");
    let mut transport = bbmd_at(&path).await;
    let table = vec![row([10, 0, 0, 3], 0xBAC0)];
    tables(&transport).activate(&bdt_changes(table)).unwrap();
    saved(&transport, 1).await;
    assert_eq!(transport.bdt_persist_failures(), 1);
    assert!(!path.exists());
    let in_use = tables(&transport).broadcast_distribution_table();
    assert!(in_use.contains(&row([10, 0, 0, 3], 0xBAC0)), "{in_use:?}");
    transport.stop().await.unwrap();
}

/// A save replaces the file whole: a shorter BDT leaves no rows of the
/// longer one behind.
#[tokio::test]
async fn a_shorter_bdt_replaces_the_file_whole() {
    let dir = TempDir::new("bdt-activation-shrink");
    let path = dir.file("bdt.bin");
    let mut transport = bbmd_at(&path).await;
    let view = tables(&transport);
    view.activate(&bdt_changes(vec![
        row([10, 0, 0, 2], 0xBAC0),
        row([10, 0, 0, 3], 0xBAC0),
    ]))
    .unwrap();
    saved(&transport, 1).await;
    let long = fs::read(&path).unwrap().len();
    view.activate(&bdt_changes(vec![row([10, 0, 0, 4], 0xBAC0)]))
        .unwrap();
    saved(&transport, 2).await;
    let short = fs::read(&path).unwrap();
    assert!(short.len() < long);
    let saved = crate::bbmd::decode_file(&short).unwrap().0;
    assert!(saved.iter().any(|entry| entry.ip == [10, 0, 0, 4]));
    assert!(!saved.iter().any(|entry| entry.ip == [10, 0, 0, 2]));
    transport.stop().await.unwrap();
}

/// Activations whose saves race: whichever thread runs last, the file ends
/// up with the newest table.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn racing_saves_leave_the_newest_table() {
    let dir = TempDir::new("bdt-activation-race");
    let path = dir.file("bdt.bin");
    let mut transport = bbmd_at(&path).await;
    let view = tables(&transport);
    for last in 1..=20u8 {
        view.activate(&bdt_changes(vec![row([10, 0, 1, last], 0xBAC0)]))
            .unwrap();
    }
    saved(&transport, 20).await;
    let rows = crate::bbmd::decode_file(&fs::read(&path).unwrap())
        .unwrap()
        .0;
    assert!(
        rows.iter().any(|entry| entry.ip == [10, 0, 1, 20]),
        "{rows:?}"
    );
    assert!(temporaries(&dir).is_empty());
    assert_eq!(transport.bdt_persist_failures(), 0);
    transport.stop().await.unwrap();
}

/// A save of an older generation than one already written is skipped.
#[test]
fn an_older_save_never_overwrites_a_newer_one() {
    let dir = TempDir::new("bdt-generation");
    let file = bbmd::BdtFile::new(dir.file("bdt.bin"));
    let (older, newer) = (file.next_generation(), file.next_generation());
    let entry = |last| bbmd::BdtEntry {
        ip: [10, 0, 2, last],
        port: 0xBAC0,
        broadcast_mask: [0xFF; 4],
    };
    file.save(newer, &[entry(2)]);
    file.save(older, &[entry(1)]);
    let rows = BbmdState::decode_bdt(&fs::read(file.path()).unwrap()).unwrap();
    assert_eq!(rows, [entry(2)]);
    assert_eq!(file.finished(), 2);
}

/// A save that never runs, or panics on its thread, counts as failed, so a
/// lost activated table is never silent.
#[test]
fn a_save_that_never_runs_or_panics_is_counted() {
    let dir = TempDir::new("bdt-lost-save");
    let file = Arc::new(bbmd::BdtFile::new(dir.file("bdt.bin")));
    drop(file.pending_save());
    assert_eq!((file.failures(), file.finished()), (1, 1));

    let pending = file.pending_save();
    let panicked = std::thread::spawn(move || {
        let _pending = pending;
        panic!("a save that dies on its thread");
    })
    .join();
    assert!(panicked.is_err());
    assert_eq!((file.failures(), file.finished()), (2, 2));

    // One that runs to the end counts only as finished.
    file.pending_save().run(file.next_generation(), &[]);
    assert_eq!((file.failures(), file.finished()), (2, 3));
    assert!(file.path().exists());
}
