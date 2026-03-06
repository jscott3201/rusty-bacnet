plugins {
    kotlin("jvm") version "2.1.0"
    id("me.champeau.jmh") version "0.7.2"
}

group = property("group") as String
version = property("version") as String

repositories {
    mavenCentral()
}

dependencies {
    jmh(project(":"))
    jmh("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
    jmh("org.openjdk.jmh:jmh-core:1.37")
    jmh("org.openjdk.jmh:jmh-generator-annprocess:1.37")
}

java {
    toolchain {
        languageVersion.set(JavaLanguageVersion.of(21))
    }
}

kotlin {
    jvmToolchain(21)
}

jmh {
    warmupIterations.set(3)
    iterations.set(5)
    fork.set(1)
    resultFormat.set("JSON")
    resultsFile.set(layout.buildDirectory.file("reports/jmh/results.json"))
}
