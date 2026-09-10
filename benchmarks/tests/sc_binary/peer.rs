//! A bounded independent wire peer: no background tasks to strand on panic.
use super::support::bounded;
use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_transport::sc::WebSocketPort;
use bacnet_transport::sc_frame::{decode_sc_message, encode_sc_message, ScFunction, ScMessage};
use bacnet_transport::sc_tls::{ScNodeTlsConfig, TlsWebSocket};
use bytes::{Bytes, BytesMut};

pub struct Peer(pub TlsWebSocket, std::cell::Cell<u8>);

impl Peer {
    pub async fn connect(url: &str, tls: ScNodeTlsConfig, id: u8) -> Self {
        Self::connect_identity(
            url,
            tls,
            id,
            super::support::HUB_VMAC,
            super::support::HUB_UUID,
        )
        .await
    }

    pub async fn connect_identity(
        url: &str,
        tls: ScNodeTlsConfig,
        id: u8,
        hub_vmac: [u8; 6],
        hub_uuid: [u8; 16],
    ) -> Self {
        let peer = Self(
            bounded(TlsWebSocket::connect(url, tls)).await.unwrap(),
            std::cell::Cell::new(7),
        );
        // Literal AB.2.10/11 vector and offsets: no product Connect codec oracle.
        let mut payload = vec![6, 0, 0, 1];
        payload.extend_from_slice(&[id; 6]);
        payload.extend_from_slice(&[id; 16]);
        payload.extend_from_slice(&[0x05, 0xc4, 0x05, 0xc4]);
        bounded(peer.0.send(&payload)).await.unwrap();
        let frame = bounded(peer.0.recv()).await.unwrap();
        assert_eq!(frame.len(), 30);
        assert_eq!(&frame[..4], &[7, 0, 0, 1]);
        assert_eq!(&frame[4..10], &hub_vmac);
        assert_eq!(&frame[10..26], &hub_uuid);
        peer
    }

    pub async fn send(&self, function: ScFunction, destination: Option<[u8; 6]>, payload: &[u8]) {
        let mut frame = BytesMut::new();
        encode_sc_message(
            &mut frame,
            &ScMessage {
                function,
                message_id: 1,
                originating_vmac: None,
                destination_vmac: destination,
                dest_options: vec![],
                data_options: vec![],
                payload: Bytes::copy_from_slice(payload),
            },
        );
        bounded(self.0.send(&frame)).await.unwrap();
    }

    pub async fn recv(&self) -> ScMessage {
        let frame = bounded(self.0.recv()).await.unwrap();
        decode_sc_message(&frame).unwrap()
    }

    pub async fn read(&self) {
        // Independent fixed NPDU + unsegmented ReadProperty vectors. Device
        // 5000 (0x02001388), AI 1, invoke ID 7, properties 75 and 85.
        for (object, property, expected) in [
            (
                [0x02, 0x00, 0x13, 0x88],
                75,
                vec![0xc4, 0x02, 0x00, 0x13, 0x88],
            ),
            (
                [0x00, 0x00, 0x00, 0x01],
                85,
                vec![0x44, 0x41, 0xa0, 0xcc, 0xcd],
            ),
        ] {
            let invoke = self.1.get();
            self.1.set(invoke.checked_add(1).unwrap());
            let mut payload = vec![0x01, 0x04, 0x00, 0x03, invoke, 0x0c, 0x0c];
            payload.extend_from_slice(&object);
            payload.extend_from_slice(&[0x19, property]);
            self.send(
                ScFunction::EncapsulatedNpdu,
                Some([2, 0, 0, 0, 0x13, 0x88]),
                &payload,
            )
            .await;
            bounded(async {
                loop {
                    let message = self.recv().await;
                    assert_eq!(message.function, ScFunction::EncapsulatedNpdu);
                    let npdu = decode_npdu(message.payload).unwrap();
                    let apdu = decode_apdu(npdu.payload).unwrap();
                    if matches!(apdu, Apdu::UnconfirmedRequest(_)) {
                        continue;
                    }
                    assert_eq!(message.originating_vmac, Some([2, 0, 0, 0, 0x13, 0x88]));
                    let Apdu::ComplexAck(ack) = apdu else {
                        panic!("expected ReadProperty ACK: {apdu:?}")
                    };
                    assert_eq!(ack.invoke_id, invoke);
                    assert_eq!(ack.service_choice.to_raw(), 12);
                    let value = ReadPropertyACK::decode(&ack.service_ack).unwrap();
                    assert_eq!(value.property_identifier.to_raw(), u32::from(property));
                    assert_eq!(value.property_value, expected);
                    return;
                }
            })
            .await;
        }
    }
}
