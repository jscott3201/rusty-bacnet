/**
 * BACnet/IPv6 client and server example.
 *
 * Demonstrates:
 * - Running a server on IPv6 with BIP6 transport
 * - Connecting a client over IPv6
 * - Same API as BIP, just a different TransportConfig variant
 */

import kotlinx.coroutines.runBlocking
import uniffi.bacnet_java.*

fun main() = runBlocking {
    // Start an IPv6 server
    val serverPort: UShort = 47873u
    val server = BacnetServer(
        deviceInstance = 5000u,
        deviceName = "IPv6 Device",
        config = TransportConfig.BipIpv6(
            address = "::1",
            port = serverPort,
        ),
    )
    server.addAnalogInput(1u, "Temp", 62u, 21.0f)
    server.start()

    val serverAddr = "::1:$serverPort"  // IPv6 address format for BIP6
    println("IPv6 server at $serverAddr")

    // Connect an IPv6 client
    val client = BacnetClient(
        config = TransportConfig.BipIpv6(
            address = "::1",
            port = 0u,
        ),
        apduTimeoutMs = 6000u,
    )
    client.connect()

    try {
        val analogInput: UInt = 0u
        val presentValue: UInt = 85u

        // Read a property over IPv6
        val value = client.readProperty(serverAddr, analogInput, 1u, presentValue, null)
        println("Read over IPv6: $value")

        // Write and verify
        client.writeProperty(
            serverAddr, analogInput, 1u, presentValue,
            BacnetPropertyValue.Real(22.5f),
            null, null,
        )
        val readback = client.readProperty(serverAddr, analogInput, 1u, presentValue, null)
        println("After write: $readback")

    } finally {
        client.stop()
        server.stop()
        println("Done.")
    }
}
