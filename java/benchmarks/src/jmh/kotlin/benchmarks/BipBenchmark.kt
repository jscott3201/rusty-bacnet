package benchmarks

import org.openjdk.jmh.annotations.*
import uniffi.bacnet_java.*
import java.util.concurrent.TimeUnit

/**
 * BIP transport benchmarks: Python client → Rust server equivalent.
 * Measures UniFFI/JNA overhead for common BACnet operations over UDP/IPv4.
 */
@State(Scope.Benchmark)
@Warmup(iterations = 3, time = 2)
@Measurement(iterations = 5, time = 3)
@Fork(1)
@BenchmarkMode(Mode.AverageTime, Mode.Throughput)
@OutputTimeUnit(TimeUnit.MICROSECONDS)
open class BipBenchmark {

    private lateinit var server: BacnetServer
    private lateinit var client: BacnetClient

    private val serverAddr = serverAddress(PORT_BIP_SERVER)

    // Object identifiers (analog-input 0, analog-output 0, binary-value 0)
    private val aiType: UInt = 0u      // analog-input
    private val aoType: UInt = 1u      // analog-output
    private val bvType: UInt = 5u      // binary-value
    private val pvProp: UInt = 85u     // present-value
    private val nameProp: UInt = 77u   // object-name

    @Setup(Level.Trial)
    fun setup() {
        server = BacnetServer(1000u, "BenchServer", bipServerConfig())
        populateServer(server)
        runSuspend { server.start() }

        client = BacnetClient(bipClientConfig(), 6000uL)
        runSuspend { client.connect() }

        // Warmup connection
        runSuspend { client.readProperty(serverAddr, aiType, 0u, pvProp, null) }
    }

    @TearDown(Level.Trial)
    fun teardown() {
        runSuspend { client.stop() }
        runSuspend { server.stop() }
    }

    @Benchmark
    fun readProperty(): BacnetPropertyValue = runSuspend {
        client.readProperty(serverAddr, aiType, 0u, pvProp, null)
    }

    @Benchmark
    fun writeProperty(): Unit = runSuspend {
        client.writeProperty(
            serverAddr, aoType, 0u, pvProp,
            BacnetPropertyValue.Real(72.5f),
            8u, null,
        )
    }

    @Benchmark
    fun readPropertyMultiple(): List<ObjectReadResult> = runSuspend {
        client.readPropertyMultiple(
            serverAddr,
            listOf(
                ReadAccessSpec(aiType, 0u, listOf(PropertyRef(pvProp, null), PropertyRef(nameProp, null))),
                ReadAccessSpec(aiType, 1u, listOf(PropertyRef(pvProp, null), PropertyRef(nameProp, null))),
                ReadAccessSpec(aiType, 2u, listOf(PropertyRef(pvProp, null), PropertyRef(nameProp, null))),
            ),
        )
    }

    @Benchmark
    fun covSubscribeUnsubscribe(): Unit = runSuspend {
        client.subscribeCov(serverAddr, 1u, aiType, 0u, false, 300u)
        client.unsubscribeCov(serverAddr, 1u, aiType, 0u)
    }

    @Benchmark
    fun whoIsDiscovery(): Unit = runSuspend {
        client.whoIs(null, null)
    }
}
