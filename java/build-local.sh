#!/usr/bin/env bash
# Build the bacnet-java JAR with native library for the current platform.
# Usage: ./build-local.sh [--release]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
JAVA_DIR="$SCRIPT_DIR"

# Determine build profile
PROFILE="release"
CARGO_FLAGS="--release"
if [[ "${1:-}" != "--release" ]]; then
    PROFILE="debug"
    CARGO_FLAGS=""
fi

echo "==> Building native library ($PROFILE)..."
cd "$ROOT_DIR"
cargo build -p bacnet-java $CARGO_FLAGS

echo "==> Generating Kotlin bindings..."
DYLIB_PATH="$ROOT_DIR/target/$PROFILE"
case "$(uname -s)" in
    Linux*)  LIB_FILE="libbacnet_java.so" ;;
    Darwin*) LIB_FILE="libbacnet_java.dylib" ;;
    MINGW*|MSYS*|CYGWIN*) LIB_FILE="bacnet_java.dll" ;;
    *) echo "Unsupported OS"; exit 1 ;;
esac

cargo run -p uniffi-bindgen -- generate \
    --library "$DYLIB_PATH/$LIB_FILE" \
    --language kotlin \
    --no-format \
    --out-dir "$JAVA_DIR/src/main/kotlin"

echo "==> Copying native library to resources..."
# Determine JNA platform directory
OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS" in
    Linux*)
        case "$ARCH" in
            x86_64)  JNA_DIR="linux-x86-64" ;;
            aarch64) JNA_DIR="linux-aarch64" ;;
            *) echo "Unsupported arch: $ARCH"; exit 1 ;;
        esac
        ;;
    Darwin*)
        case "$ARCH" in
            x86_64)  JNA_DIR="darwin-x86-64" ;;
            arm64)   JNA_DIR="darwin-aarch64" ;;
            *) echo "Unsupported arch: $ARCH"; exit 1 ;;
        esac
        ;;
    MINGW*|MSYS*|CYGWIN*)
        JNA_DIR="win32-x86-64"
        ;;
esac

RESOURCE_DIR="$JAVA_DIR/src/main/resources/$JNA_DIR"
mkdir -p "$RESOURCE_DIR"
cp "$DYLIB_PATH/$LIB_FILE" "$RESOURCE_DIR/"

echo "==> Building JAR..."
cd "$JAVA_DIR"
./gradlew build

echo "==> Done! JAR at: java/build/libs/"
ls -lh build/libs/*.jar 2>/dev/null || true
