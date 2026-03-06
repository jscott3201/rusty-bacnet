/**
 * Device management example.
 *
 * Demonstrates:
 * - DeviceCommunicationControl (enable/disable)
 * - CreateObject / DeleteObject
 * - Error handling with typed exceptions
 */

import kotlinx.coroutines.runBlocking
import uniffi.bacnet_java.*

fun main() = runBlocking {
    // --- Server ---
    val serverPort: UShort = 47872u
    val server = BacnetServer(
        deviceInstance = 3000u,
        deviceName = "Managed Device",
        config = TransportConfig.Bip(
            address = "127.0.0.1",
            port = serverPort,
            broadcastAddress = "127.0.0.255",
        ),
    )
    server.addAnalogInput(1u, "Temp", 62u, 72.0f)
    server.start()

    val serverAddr = "127.0.0.1:$serverPort"

    // --- Client ---
    val client = BacnetClient(
        config = TransportConfig.Bip(
            address = "127.0.0.1",
            port = 0u,
            broadcastAddress = "127.0.0.255",
        ),
        apduTimeoutMs = 6000u,
    )
    client.connect()

    try {
        val analogInput: UInt = 0u
        val presentValue: UInt = 85u

        // --- Error Handling ---
        println("=== Error Handling ===")
        try {
            // Try reading a non-existent property (proprietary range)
            client.readProperty(serverAddr, analogInput, 1u, 9999u, null)
        } catch (e: BacnetError.ProtocolError) {
            println("Protocol error (expected): ${e.msg}")
        } catch (e: BacnetError.Timeout) {
            println("Timeout (device not responding)")
        } catch (e: BacnetError) {
            println("General BACnet error: $e")
        }

        // --- CreateObject ---
        println("\n=== CreateObject ===")
        val analogValue: UInt = 2u  // ObjectType::AnalogValue
        val objectName: UInt = 77u
        val unitsProperty: UInt = 117u
        val rawAck = client.createObject(
            serverAddr,
            analogValue,
            listOf(
                PropertyWrite(objectName, null, BacnetPropertyValue.CharacterString("Dynamic AV"), null),
                PropertyWrite(unitsProperty, null, BacnetPropertyValue.Enumerated(62u), null),
            ),
        )
        println("Created object (raw ACK: ${rawAck.size} bytes)")

        // Verify it exists by reading its name
        val name = client.readProperty(serverAddr, analogValue, 1u, objectName, null)
        println("Read back object-name: $name")

        // --- DeviceCommunicationControl ---
        println("\n=== DeviceCommunicationControl ===")
        client.deviceCommunicationControl(
            serverAddr,
            enableDisable = 1u,    // 1 = Disable
            timeDuration = 1u,     // 1 minute
            password = null,
        )
        println("Device communication disabled")

        // Re-enable
        client.deviceCommunicationControl(
            serverAddr,
            enableDisable = 0u,    // 0 = Enable
            timeDuration = null,
            password = null,
        )
        println("Device communication re-enabled")

        // --- DeleteObject ---
        println("\n=== DeleteObject ===")
        try {
            client.deleteObject(serverAddr, analogValue, 1u)
            println("Object deleted")
        } catch (e: BacnetError) {
            println("Delete failed: $e")
        }

    } finally {
        client.stop()
        server.stop()
        println("\nDone.")
    }
}
