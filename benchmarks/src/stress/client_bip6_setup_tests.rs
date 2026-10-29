use super::*;
use std::cell::Cell;
use std::net::Ipv6Addr;
use std::rc::Rc;

struct Owner {
    index: usize,
    dropped: Rc<Cell<usize>>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.dropped.set(self.dropped.get() + 1);
    }
}

async fn fails_at(index: usize, during_resolution: bool) {
    let dropped = Rc::new(Cell::new(0));
    let mut next = 0;
    let mut clients = Vec::new();
    let mut report = StepReport::new(0, 4);
    let mac = setup(
        Ok(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 47808, 0, 0)),
        4,
        &mut clients,
        async || {
            let attempted = next;
            next += 1;
            if !during_resolution && attempted == index {
                return Err(Error::Encoding("injected build failure".into()));
            }
            Ok(Owner {
                index: attempted,
                dropped: Rc::clone(&dropped),
            })
        },
        async |owner: &Owner, _| {
            if during_resolution && owner.index == index {
                Err(Error::Encoding("injected resolution failure".into()))
            } else {
                Ok(vec![1, 2, 3])
            }
        },
        &mut report,
    )
    .await;
    assert!(mac.is_none());
    let retained = index + usize::from(during_resolution);
    assert_eq!(
        clients.iter().map(|owner| owner.index).collect::<Vec<_>>(),
        (0..retained).collect::<Vec<_>>()
    );
    assert_eq!(
        dropped.get(),
        0,
        "setup must retain owners for caller cleanup"
    );
    assert_eq!(report.diagnostics.started_clients, retained);
    let failure = &report.diagnostics.lifecycle_failures[0];
    assert_eq!(failure.stage, "client_setup");
    assert_eq!(failure.client, Some(index));
    report.finish();
    assert!(!report.diagnostics.valid);
    drop(clients);
    assert_eq!(dropped.get(), retained);
}

#[tokio::test(start_paused = true)]
async fn bip6_setup_first_build_failure() {
    fails_at(0, false).await;
}
#[tokio::test(start_paused = true)]
async fn bip6_setup_later_build_failure() {
    fails_at(2, false).await;
}
#[tokio::test(start_paused = true)]
async fn bip6_setup_first_resolution_failure() {
    fails_at(0, true).await;
}
#[tokio::test(start_paused = true)]
async fn bip6_setup_later_resolution_failure() {
    fails_at(2, true).await;
}

#[tokio::test(start_paused = true)]
async fn bip6_setup_server_address_failure_has_no_client() {
    let mut clients: Vec<()> = Vec::new();
    let mut report = StepReport::new(0, 1);
    let result = setup(
        Err(Error::Encoding("injected server address failure".into())),
        1,
        &mut clients,
        async || -> Result<(), Error> { panic!("must not build a client") },
        async |_: &(), _| -> Result<Vec<u8>, Error> { panic!("must not resolve a client") },
        &mut report,
    )
    .await;
    assert!(result.is_none());
    assert!(clients.is_empty());
    assert_eq!(report.diagnostics.started_clients, 0);
    let failure = &report.diagnostics.lifecycle_failures[0];
    assert_eq!(failure.stage, "server_setup");
    assert_eq!(failure.client, None);
    report.finish();
    assert!(!report.diagnostics.valid);
}
