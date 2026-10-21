use super::*;

#[derive(Debug)]
struct Reject;

impl ServerCertVerifier for Reject {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Err(Error::General("original rejection".into()))
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        unreachable!()
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        unreachable!()
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![]
    }
}

fn reject(bytes: &[u8]) {
    let error = ObservingVerifier(Arc::new(Reject))
        .verify_server_cert(
            &CertificateDer::from(bytes),
            &[],
            &ServerName::try_from("localhost").unwrap(),
            &[],
            UnixTime::since_unix_epoch(std::time::Duration::ZERO),
        )
        .unwrap_err();
    assert_eq!(error, Error::General("original rejection".into()));
}

#[tokio::test]
async fn nested_and_cancelled_scopes_restore_the_previous_observation() {
    reject(b"outside"); // No active scope still delegates and rejects.
    let (_, outer) = capture(async {
        reject(b"outer");
        let (_, inner) = capture(async {
            tokio::task::yield_now().await;
            reject(b"inner");
        })
        .await;
        assert_eq!(inner.unwrap(), vec![b"inner".to_vec()]);
        {
            let cancelled = capture(async {
                reject(b"cancelled");
                std::future::pending::<()>().await;
            });
            tokio::pin!(cancelled);
            tokio::select! {
                biased;
                _ = &mut cancelled => panic!("pending capture completed"),
                _ = std::future::ready(()) => {}
            }
        }
        assert_eq!(
            OBSERVED.with(|data| data.borrow().clone()).unwrap(),
            vec![b"outer".to_vec()]
        );
    })
    .await;
    assert_eq!(outer.unwrap(), vec![b"outer".to_vec()]);
    assert!(OBSERVED.try_with(|_| ()).is_err());
    assert!(capture(async {}).await.1.is_none());
}

#[tokio::test]
async fn interleaved_futures_on_one_task_do_not_mix_certificates() {
    let left = capture(async {
        reject(b"left");
        tokio::task::yield_now().await;
    });
    let right = capture(async {
        tokio::task::yield_now().await;
        reject(b"right");
    });
    let (((), left), ((), right)) = tokio::join!(left, right);
    assert_eq!(left.unwrap(), vec![b"left".to_vec()]);
    assert_eq!(right.unwrap(), vec![b"right".to_vec()]);
    assert!(OBSERVED.try_with(|_| ()).is_err());
}
