//! The one `TransportPort` double the crate's unit tests share (#904): the
//! workspace's shared test transport, which grew out of this crate's own
//! (#2557). `BACnetServer<T>` and `NetworkLayer<T>` are generic over their
//! transport, so every distinct test transport type compiles the whole server
//! again; in-crate tests use this one concrete type.

pub(crate) use bacnet_transport::test_transport::{
    BIP_LOCAL_MAC, SendLog, SendMode, SentFrame, StartMode, TestTransport, TestTransportHandle,
};
