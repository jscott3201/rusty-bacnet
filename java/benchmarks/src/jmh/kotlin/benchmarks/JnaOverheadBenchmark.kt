package benchmarks

import org.openjdk.jmh.annotations.*
import uniffi.bacnet_java.*
import java.util.concurrent.TimeUnit

/**
 * JNA/UniFFI overhead benchmarks: isolate the FFI call cost without network I/O.
 * Measures raw cost of crossing the Kotlin → JNA → Rust boundary.
 */
@State(Scope.Benchmark)
@Warmup(iterations = 3, time = 2)
@Measurement(iterations = 5, time = 3)
@Fork(1)
@BenchmarkMode(Mode.AverageTime, Mode.Throughput)
@OutputTimeUnit(TimeUnit.NANOSECONDS)
open class JnaOverheadBenchmark {

    private lateinit var cachedId: BacnetObjectIdentifier

    @Setup(Level.Trial)
    fun setup() {
        cachedId = BacnetObjectIdentifier(0u, 1u)
    }

    @Benchmark
    fun objectIdentifierCreate(): BacnetObjectIdentifier =
        BacnetObjectIdentifier(0u, 1u)

    @Benchmark
    fun propertyValueCreateReal(): BacnetPropertyValue =
        BacnetPropertyValue.Real(72.5f)

    @Benchmark
    fun propertyValueCreateUnsigned(): BacnetPropertyValue =
        BacnetPropertyValue.Unsigned(42u)

    @Benchmark
    fun propertyValueCreateString(): BacnetPropertyValue =
        BacnetPropertyValue.CharacterString("Zone Temp")

    @Benchmark
    fun objectIdentifierDisplay(): String =
        cachedId.display()
}

/**
 * Server object creation throughput: measures how fast objects can be registered.
 */
@State(Scope.Benchmark)
@Warmup(iterations = 3, time = 2)
@Measurement(iterations = 5, time = 3)
@Fork(1)
@BenchmarkMode(Mode.AverageTime, Mode.Throughput)
@OutputTimeUnit(TimeUnit.MICROSECONDS)
open class ObjectCreationBenchmark {

    private var instanceCounter: UInt = 0u

    // Use a unique port per invocation to avoid bind conflicts
    private var portCounter: UShort = 47870u

    @Setup(Level.Invocation)
    fun setup() {
        instanceCounter = 0u
    }

    @Benchmark
    fun addAnalogInput(): Unit {
        val server = BacnetServer(
            3000u + instanceCounter,
            "ObjBench",
            bipServerConfig((portCounter++).toUShort()),
        )
        server.addAnalogInput(instanceCounter++, "AI-bench", UNITS_DEGREES_F, 72.5f)
    }

    @Benchmark
    fun addMixedObjects(): Unit {
        val port = (portCounter++).toUShort()
        val server = BacnetServer(
            4000u + instanceCounter,
            "MixedBench",
            bipServerConfig(port),
        )
        // 3 analog inputs + 3 analog outputs + 3 binary values + 1 multistate input
        for (i in 0u until 3u) {
            server.addAnalogInput(i, "AI-$i", UNITS_DEGREES_F, 72.5f)
            server.addAnalogOutput(i, "AO-$i", UNITS_DEGREES_F)
            server.addBinaryValue(i, "BV-$i")
        }
        server.addMultistateInput(0u, "MI-0", 3u)
        instanceCounter += 10u
    }
}
