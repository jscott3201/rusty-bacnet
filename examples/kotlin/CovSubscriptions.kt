/**
 * Change of Value (COV) subscription example.
 *
 * Demonstrates:
 * - Subscribing to COV notifications on an analog input
 * - Receiving real-time value changes via coroutine
 * - Server-side writes triggering COV notifications
 * - Unsubscribing when done
 */

import kotlinx.coroutines.*
import uniffi.bacnet_java.*

fun main() = runBlocking {
    // --- Server ---
    val serverPort: UShort = 47871u
    val server = BacnetServer(
        deviceInstance = 1000u,
        deviceName = "Sensor Controller",
        config = TransportConfig.Bip(
            address = "127.0.0.1",
            port = serverPort,
            broadcastAddress = "127.0.0.255",
        ),
    )
    server.addAnalogInput(1u, "Zone Temp", 62u, 72.0f)
    server.start()

    val serverAddr = "127.0.0.1:$serverPort"
    println("Server at $serverAddr")

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

        // Subscribe to COV (confirmed, 60-second lifetime)
        client.subscribeCov(
            serverAddr,
            subscriberProcessIdentifier = 1u,
            objectType = analogInput,
            objectInstance = 1u,
            confirmed = true,
            lifetime = 60u,
        )
        println("Subscribed to COV on AnalogInput:1")

        // Start a listener coroutine
        val stream = client.covNotifications()
        val listener = launch {
            while (isActive) {
                val notif = stream.next() ?: break
                println("\n  COV notification:")
                println("    device=${notif.deviceInstance} object=AI:${notif.objectInstance}")
                for (v in notif.values) {
                    println("    property ${v.propertyId}: ${v.value}")
                }
            }
        }

        // Simulate value changes on the server
        for (temp in listOf(73.0f, 74.5f, 71.0f, 75.0f)) {
            delay(300)
            server.writePropertyLocal(
                analogInput, 1u,
                presentValue,
                BacnetPropertyValue.Real(temp),
            )
            println("Server wrote: $temp")
        }

        delay(500)

        // Unsubscribe
        client.unsubscribeCov(
            serverAddr,
            subscriberProcessIdentifier = 1u,
            objectType = analogInput,
            objectInstance = 1u,
        )
        println("\nUnsubscribed from COV")
        listener.cancel()

    } finally {
        client.stop()
        server.stop()
    }
}
