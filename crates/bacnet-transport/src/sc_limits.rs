//! Local BACnet/SC receive and WebSocket resource budgets.

// Annex AB.5.1 hub workload: 16-byte envelope + 4192 encoded option bytes
// + 1497-byte NPDU. Nodes use this full-message default independently of NPDU.
pub(crate) const DEFAULT_MAX_BVLC_LENGTH: u16 = 5705;

#[cfg(feature = "sc-tls")]
pub(crate) fn websocket(
    max_receive: usize,
) -> tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
    // Outbound peer capacity is independent of our receive limit. Permit two
    // full u16-sized messages plus worst-case framing, with a bounded queue.
    tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .read_buffer_size(4096)
        .write_buffer_size(4096)
        .max_write_buffer_size(2 * (u16::MAX as usize + 14))
        .max_frame_size(Some(max_receive))
        .max_message_size(Some(max_receive))
}

#[cfg(all(test, feature = "sc-tls"))]
mod tests {
    use super::*;
    use std::io::{self, Read, Write};
    use tokio_tungstenite::tungstenite::{protocol::Role, Error, Message, WebSocket};

    struct Blocked;
    impl Read for Blocked {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::WouldBlock.into())
        }
    }
    impl Write for Blocked {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::WouldBlock.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::ErrorKind::WouldBlock.into())
        }
    }

    #[test]
    fn websocket_write_queue_is_bounded_for_blocked_io_and_full_peer_capacity() {
        for (role, receive_limit) in [(Role::Server, 5705), (Role::Client, 65535)] {
            let config = websocket(receive_limit);
            assert_eq!(
                (config.read_buffer_size, config.write_buffer_size),
                (4096, 4096)
            );
            assert_eq!(config.max_write_buffer_size, 131098);
            assert!(!config.accept_unmasked_frames);
            let mut socket = WebSocket::from_raw_socket(Blocked, role, Some(config));
            let message = Message::Binary(vec![0x55; 65535].into());
            for _ in 0..2 {
                assert!(
                    matches!(socket.write(message.clone()), Err(Error::Io(error)) if error.kind() == io::ErrorKind::WouldBlock)
                );
            }
            assert!(matches!(
                socket.write(message),
                Err(Error::WriteBufferFull(_))
            ));
        }
    }
}
