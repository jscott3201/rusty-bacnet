package benchmarks

// SC (BACnet/SC over TLS WebSocket) benchmarks are not yet available
// for the Java/Kotlin bindings because the ScHub is not yet exposed
// via UniFFI. Once ScHub is added to bacnet-java, this file will
// mirror BipBenchmark with SC transport configuration.
//
// To add SC benchmarks:
// 1. Expose ScHub in crates/bacnet-java/src/lib.rs
// 2. Use TransportConfig.Sc with hub_url, ca_cert, client_cert, client_key
// 3. Use generateTlsCerts() from Helpers.kt for test certificates
