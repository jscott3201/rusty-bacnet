@file:Suppress("unused")

package benchmarks

import kotlinx.coroutines.runBlocking
import uniffi.bacnet_java.*
import java.io.File
import java.nio.file.Files

// Engineering units
const val UNITS_DEGREES_F: UInt = 64u
const val UNITS_PERCENT: UInt = 23u

// Ports — offset from Python benchmarks to avoid conflicts
// BIP: 47840-47859, SC: 47950-47969
const val PORT_BIP_SERVER: UShort = 47840u
const val PORT_BIP_CLIENT: UShort = 47841u

const val PORT_SC_HUB: UShort = 47950u
const val PORT_SC_SERVER: UShort = 47951u
const val PORT_SC_CLIENT: UShort = 47952u

// Concurrency benchmark uses ports 47843-47870
const val PORT_CONCURRENCY_SERVER: UShort = 47843u
const val PORT_CONCURRENCY_CLIENT_BASE: UShort = 47844u

fun bipServerConfig(port: UShort = PORT_BIP_SERVER): TransportConfig =
    TransportConfig.Bip(
        address = "127.0.0.1",
        port = port,
        broadcastAddress = "127.0.0.255",
    )

fun bipClientConfig(port: UShort = PORT_BIP_CLIENT): TransportConfig =
    TransportConfig.Bip(
        address = "127.0.0.1",
        port = port,
        broadcastAddress = "127.0.0.255",
    )

fun populateServer(server: BacnetServer) {
    server.addAnalogInput(0u, "AI-0", UNITS_DEGREES_F, 72.5f)
    server.addAnalogInput(1u, "AI-1", UNITS_DEGREES_F, 68.0f)
    server.addAnalogInput(2u, "AI-2", UNITS_PERCENT, 55.0f)
    server.addAnalogOutput(0u, "AO-0", UNITS_DEGREES_F)
    server.addBinaryValue(0u, "BV-0")
}

fun serverAddress(port: UShort = PORT_BIP_SERVER): String = "127.0.0.1:$port"

/** Run a suspend block synchronously for JMH. */
fun <T> runSuspend(block: suspend () -> T): T = runBlocking { block() }

data class TlsCerts(
    val caCert: String,
    val serverCert: String,
    val serverKey: String,
    val clientCert: String,
    val clientKey: String,
    val tmpDir: File,
) {
    fun cleanup() {
        tmpDir.deleteRecursively()
    }
}

fun generateTlsCerts(): TlsCerts {
    val tmpDir = Files.createTempDirectory("bacnet_bench_certs_").toFile()

    fun run(cmd: String) {
        val proc = ProcessBuilder("bash", "-c", cmd)
            .directory(tmpDir)
            .redirectErrorStream(true)
            .start()
        proc.waitFor()
        check(proc.exitValue() == 0) {
            "openssl command failed: ${proc.inputStream.bufferedReader().readText()}"
        }
    }

    // CA
    run(
        "openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 " +
            "-keyout ca.key -out ca.pem -days 1 -nodes " +
            "-subj '/CN=BACnet Bench CA'"
    )

    // SAN config
    File(tmpDir, "san.cnf").writeText(
        """
        [req]
        distinguished_name = req_dn
        req_extensions = v3_req
        [req_dn]
        [v3_req]
        subjectAltName = DNS:localhost,IP:127.0.0.1
        [v3_ca]
        subjectAltName = DNS:localhost,IP:127.0.0.1
        """.trimIndent()
    )

    // Server cert
    run(
        "openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 " +
            "-keyout server.key -out server.csr -nodes " +
            "-subj '/CN=localhost' -config san.cnf"
    )
    run(
        "openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key " +
            "-CAcreateserial -out server.pem -days 1 " +
            "-extensions v3_ca -extfile san.cnf"
    )

    // Client cert
    run(
        "openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 " +
            "-keyout client.key -out client.csr -nodes " +
            "-subj '/CN=BACnet Bench Client'"
    )
    run(
        "openssl x509 -req -in client.csr -CA ca.pem -CAkey ca.key " +
            "-CAcreateserial -out client.pem -days 1"
    )

    return TlsCerts(
        caCert = File(tmpDir, "ca.pem").absolutePath,
        serverCert = File(tmpDir, "server.pem").absolutePath,
        serverKey = File(tmpDir, "server.key").absolutePath,
        clientCert = File(tmpDir, "client.pem").absolutePath,
        clientKey = File(tmpDir, "client.key").absolutePath,
        tmpDir = tmpDir,
    )
}
