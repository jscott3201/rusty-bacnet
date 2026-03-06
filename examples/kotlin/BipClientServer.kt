/**
 * BACnet/IP client and server example.
 *
 * Demonstrates:
 * - Starting a BACnet/IP server with multiple object types
 * - Reading and writing properties from a client
 * - ReadPropertyMultiple for efficient bulk reads
 * - Device discovery via WhoIs/IAm
 *
 * Usage:
 *   // Add rusty-bacnet JAR to classpath, then:
 *   kotlinc -cp rusty-bacnet-0.5.4.jar BipClientServer.kt -include-runtime -d example.jar
 *   java -cp example.jar:rusty-bacnet-0.5.4.jar BipClientServerKt
 */

import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import uniffi.bacnet_java.*

fun main() = runBlocking {
    // --- Server Setup ---
    val serverPort: UShort = 47870u
    val server = BacnetServer(
        deviceInstance = 1234u,
        deviceName = "Example HVAC Controller",
        config = TransportConfig.Bip(
            address = "127.0.0.1",
            port = serverPort,
            broadcastAddress = "127.0.0.255",
        ),
    )

    // Add various object types
    server.addAnalogInput(1u, "Zone Temp", 62u, 72.5f)          // degrees-fahrenheit
    server.addAnalogOutput(1u, "Damper Position", 98u)           // percent
    server.addAnalogValue(1u, "Cooling Setpoint", 62u)
    server.addBinaryInput(1u, "Occupancy Sensor")
    server.addBinaryOutput(1u, "Fan Enable")
    server.addBinaryValue(1u, "Override Mode")
    server.addMultistateValue(1u, "Operating Mode", 4u)

    server.start()
    val serverAddr = "127.0.0.1:$serverPort"
    println("Server running at $serverAddr")

    // --- Client Setup ---
    val client = BacnetClient(
        config = TransportConfig.Bip(
            address = "127.0.0.1",
            port = 0u,  // auto-assign client port
            broadcastAddress = "127.0.0.255",
        ),
        apduTimeoutMs = 6000u,
    )
    client.connect()

    try {
        // Object type and property constants
        val analogInput: UInt = 0u       // ObjectType::AnalogInput
        val analogOutput: UInt = 1u      // ObjectType::AnalogOutput
        val presentValue: UInt = 85u     // PropertyIdentifier::PresentValue
        val objectName: UInt = 77u       // PropertyIdentifier::ObjectName
        val units: UInt = 117u           // PropertyIdentifier::Units

        // Read a single property
        val value = client.readProperty(serverAddr, analogInput, 1u, presentValue, null)
        println("\nZone Temp: $value")

        // Write a property with priority
        client.writeProperty(
            serverAddr, analogOutput, 1u, presentValue,
            BacnetPropertyValue.Real(72.0f),
            8u,   // priority 8
            null,  // no array index
        )
        println("Wrote damper position: 72.0 @ priority 8")

        // Read it back
        val readback = client.readProperty(serverAddr, analogOutput, 1u, presentValue, null)
        println("Damper Position readback: $readback")

        // Read multiple properties from multiple objects
        val results = client.readPropertyMultiple(
            serverAddr,
            listOf(
                ReadAccessSpec(analogInput, 1u, listOf(
                    PropertyRef(presentValue, null),
                    PropertyRef(objectName, null),
                    PropertyRef(units, null),
                )),
                ReadAccessSpec(analogOutput, 1u, listOf(
                    PropertyRef(presentValue, null),
                    PropertyRef(objectName, null),
                )),
            ),
        )

        println("\nReadPropertyMultiple results:")
        for (obj in results) {
            println("  Object: type=${obj.objectType}, instance=${obj.instance}")
            for (prop in obj.results) {
                if (prop.value != null) {
                    println("    property ${prop.propertyId}: ${prop.value}")
                } else {
                    println("    property ${prop.propertyId}: ERROR class=${prop.errorClass} code=${prop.errorCode}")
                }
            }
        }

        // Discover devices via WhoIs
        client.whoIs(null, null)
        delay(500)
        val devices = client.discoveredDevices()
        println("\nDiscovered ${devices.size} device(s):")
        for (dev in devices) {
            println("  Device ${dev.instance} vendor=${dev.vendorId} maxApdu=${dev.maxApduLength}")
        }

    } finally {
        client.stop()
        server.stop()
        println("\nDone.")
    }
}
