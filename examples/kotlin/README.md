# Kotlin Examples

These examples demonstrate using `rusty-bacnet` from Kotlin/JVM via UniFFI bindings.

## Prerequisites

Build the JAR (includes native library and Kotlin bindings):

```bash
cd ../../java
./build-local.sh --release
```

The JAR will be at `java/build/libs/rusty-bacnet-0.6.2.jar`.

## Examples

| Example | Description |
|---------|-------------|
| [`BipClientServer.kt`](BipClientServer.kt) | BACnet/IP client and server — read, write, RPM, discovery |
| [`CovSubscriptions.kt`](CovSubscriptions.kt) | COV subscription and real-time notifications |
| [`Ipv6ClientServer.kt`](Ipv6ClientServer.kt) | BACnet/IPv6 client and server |
| [`DeviceManagement.kt`](DeviceManagement.kt) | DeviceCommunicationControl, CreateObject, error handling |

## Running

These examples use the Kotlin scripting approach. Ensure JDK 21+ and `kotlinc` are available.

```bash
# Option 1: Run via Gradle (recommended — handles classpath automatically)
# Add example as a mainClass in java/build.gradle.kts

# Option 2: Compile and run directly
JAR=../../java/build/libs/rusty-bacnet-0.6.2.jar
kotlinc -cp "$JAR" BipClientServer.kt -include-runtime -d example.jar
java -cp "example.jar:$JAR" BipClientServerKt

# Option 3: Use Kotlin script mode (kotlin 1.9+)
kotlin -cp "$JAR" BipClientServer.kt
```

## API Notes

- **Transport config**: Use `TransportConfig.Bip(...)`, `TransportConfig.BipIpv6(...)`, or `TransportConfig.Sc(...)`.
- **Async methods**: `start()`, `connect()`, `readProperty()`, etc. are `suspend` functions — call from a coroutine scope or `runBlocking { }`.
- **Object/property IDs**: Use raw `UInt` values (e.g., `0u` for AnalogInput, `85u` for PresentValue). See the [BACnet spec](https://www.ashrae.org/technical-resources/bookstore/bacnet) for standard values.
- **Property values**: Use `BacnetPropertyValue.Real(...)`, `.Unsigned(...)`, `.CharacterString(...)`, etc.
- **Error handling**: Catch `BacnetError.ProtocolError`, `BacnetError.Timeout`, or the base `BacnetError`.
