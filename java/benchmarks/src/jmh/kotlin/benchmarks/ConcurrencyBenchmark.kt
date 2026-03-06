package benchmarks

import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import org.openjdk.jmh.annotations.*
import uniffi.bacnet_java.*
import java.util.concurrent.TimeUnit

/**
 * Concurrency scaling benchmark: N parallel coroutines doing ReadProperty.
 * Measures throughput scaling as concurrent client count increases.
 */
@State(Scope.Benchmark)
@Warmup(iterations = 3, time = 2)
@Measurement(iterations = 5, time = 3)
@Fork(1)
@BenchmarkMode(Mode.Throughput)
@OutputTimeUnit(TimeUnit.SECONDS)
open class ConcurrencyBenchmark {

    @Param("1", "5", "10", "25")
    var concurrency: Int = 1

    private lateinit var server: BacnetServer
    private lateinit var clients: List<BacnetClient>

    private val serverAddr = serverAddress(PORT_CONCURRENCY_SERVER)
    private val aiType: UInt = 0u
    private val pvProp: UInt = 85u

    @Setup(Level.Trial)
    fun setup() {
        server = BacnetServer(2000u, "ConcurrencyServer", bipServerConfig(PORT_CONCURRENCY_SERVER))
        populateServer(server)
        runSuspend { server.start() }

        // Create one client per max concurrency level
        clients = (0 until 25).map { i ->
            val port = (PORT_CONCURRENCY_CLIENT_BASE.toInt() + i).toUShort()
            val c = BacnetClient(bipClientConfig(port), 6000uL)
            runSuspend { c.connect() }
            // Warmup
            runSuspend { c.readProperty(serverAddr, aiType, 0u, pvProp, null) }
            c
        }
    }

    @TearDown(Level.Trial)
    fun teardown() {
        clients.forEach { c -> runSuspend { c.stop() } }
        runSuspend { server.stop() }
    }

    @Benchmark
    fun concurrentReadProperty(): Unit = runSuspend {
        coroutineScope {
            (0 until concurrency).map { i ->
                async {
                    clients[i].readProperty(serverAddr, aiType, 0u, pvProp, null)
                }
            }.awaitAll()
        }
    }
}
