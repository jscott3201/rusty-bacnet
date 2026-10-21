#!/usr/bin/env bash
# Build the Linux release artifacts for one architecture (#1472):
#
#   PYTHONS="3.11 3.12 3.13 3.14" scripts/release/build_linux.sh <x86_64|aarch64>
#
# On the runner it runs itself in the manylinux2014 image pinned in
# .github/ci-pins.env (CentOS 7: glibc 2.17, GCC 10), with the checkout at
# /io. The image is for the runner's own architecture: the arm64 runner builds
# aarch64, and nothing is cross-compiled or emulated. In the image it:
# 1. installs the toolchain that rust-toolchain.toml pins, through a
#    rustup-init checked against its pinned SHA-256;
# 2. builds libpcap (libpcap.sh) as a static, position-independent archive.
#    The pcap crate links -lpcap, and with only libpcap.a in LIBPCAP_LIBDIR the
#    linker takes the archive, so the CLI needs no libpcap on the user's
#    system: Debian and Ubuntu name the shared library libpcap.so.0.8 and RHEL
#    libpcap.so.1, so no single dynamically linked binary runs on both.
#    LIBPCAP_VER tells the crate's build script the version, which it would
#    otherwise load a shared libpcap to ask;
# 3. builds a wheel for each CPython in PYTHONS into dist/, with the maturin
#    wheel that maturin-requirements.txt pins by version and sha256:
#    --compatibility manylinux2014 tags them manylinux_2_17,
#    and --auditwheel check fails the build on a symbol or library outside that
#    policy instead of copying a library into the wheel;
# 4. builds the CLI with sc-tls and pcap into out/bacnet-linux-<amd64|arm64>.
# The wheels and the CLI use the `dist` profile (fat LTO, #973; Cargo.toml).
# Then it gives dist/, out/ and target/ back to the runner's user.
# SOURCE_DATE_EPOCH, when set, pins the wheels' timestamps.
set -euo pipefail

fail() { echo "error: build_linux.sh: $*" >&2; exit 1; }
[ $# -eq 1 ] || fail "usage: build_linux.sh <x86_64|aarch64>"
arch=$1
case $arch in
  x86_64) cli=bacnet-linux-amd64 pin_arch=X86_64 ;;
  aarch64) cli=bacnet-linux-arm64 pin_arch=AARCH64 ;;
  *) fail "unknown architecture $arch" ;;
esac
target=$arch-unknown-linux-gnu
pythons=${PYTHONS:?set PYTHONS to the CPython versions to build wheels for}
root=$(cd "$(dirname "$0")/../.." && pwd)
pins=$root/.github/ci-pins.env

# pin NAME: the value of NAME= in the pins file, which must be there once.
pin() {
  local value
  value=$(sed -n "s/^$1=//p" "$pins")
  [[ $value =~ ^[0-9A-Za-z.]+$ ]] || fail "no single $1 in $pins"
  printf '%s\n' "$value"
}

if [ "${RELEASE_IN_IMAGE:-}" != 1 ]; then
  # The image must run natively: Docker's own architecture, not an emulator's.
  docker_arch=$(docker info --format '{{.Architecture}}')
  [ "$docker_arch" = "$arch" ] || fail "build $arch where Docker runs on $arch, not $docker_arch"
  image=quay.io/pypa/manylinux2014_$arch@sha256:$(pin "MANYLINUX2014_${pin_arch}_SHA256")
  echo "building in $image"
  exec docker run --rm -v "$root:/io" -w /io \
    -e RELEASE_IN_IMAGE=1 -e PYTHONS -e SOURCE_DATE_EPOCH -e CARGO_TERM_COLOR -e CARGO_INCREMENTAL \
    -e HOST_IDS="$(id -u):$(id -g)" \
    "$image" bash scripts/release/build_linux.sh "$arch"
fi

cd "$root"
trap 'chown -R "$HOST_IDS" dist out target 2>/dev/null || true' EXIT

echo "::group::Rust toolchain"
channel=$(sed -n 's/^channel = "\([^"]*\)"$/\1/p' rust-toolchain.toml)
components=$(sed -n 's/^components = \[\(.*\)\]$/\1/p' rust-toolchain.toml | tr -d '" ')
[[ $channel =~ ^[0-9A-Za-z.-]+$ && $components =~ ^[a-z,-]+$ ]] \
  || fail "no channel or components line in rust-toolchain.toml"
curl -sSfL --retry 3 -o /tmp/rustup-init \
  "https://static.rust-lang.org/rustup/archive/$(pin RUSTUP_VERSION)/$target/rustup-init"
echo "$(pin "RUSTUP_${pin_arch}_SHA256")  /tmp/rustup-init" | sha256sum -c --quiet - \
  || fail "rustup-init does not match its pinned SHA-256"
chmod +x /tmp/rustup-init
/tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain none
export PATH=$HOME/.cargo/bin:$PATH
rustup toolchain install "$channel" --profile minimal --component "$components"
rustc -vV
gcc --version | sed -n 1p
ldd --version | sed -n 1p
echo "::endgroup::"

# flex and bison generate libpcap's filter parser; nothing else uses them.
echo "::group::libpcap"
yum install -y -q flex bison
bash scripts/release/libpcap.sh /tmp/libpcap
(
  cd /tmp/libpcap/src
  CFLAGS="-O2 -fPIC" ./configure --quiet --disable-shared --with-pcap=linux --without-libnl \
    --disable-dbus --disable-rdma --disable-bluetooth --disable-usb --disable-netmap
  make -s -j"$(nproc)" libpcap.a
)
mkdir -p /tmp/libpcap/lib
cp /tmp/libpcap/src/libpcap.a /tmp/libpcap/lib/
echo "::endgroup::"

echo "::group::Wheels (CPython $pythons)"
maturin_python=/opt/python/cp312-cp312/bin/python
"$maturin_python" -m pip install -q --disable-pip-version-check --root-user-action=ignore \
  --require-hashes --only-binary :all: -r scripts/release/maturin-requirements.txt
"$(dirname "$maturin_python")/maturin" --version
interpreters=()
for py in $pythons; do
  tag=cp${py/./}
  interpreter=/opt/python/$tag-$tag/bin/python
  [ -x "$interpreter" ] || fail "the image has no CPython $py ($interpreter)"
  interpreters+=("$interpreter")
done
"$(dirname "$maturin_python")/maturin" build --profile dist --locked --target "$target" \
  --compatibility manylinux2014 --auditwheel check -i "${interpreters[@]}" \
  -m crates/rusty-bacnet/Cargo.toml --out dist
echo "::endgroup::"

# The default features add the terminal UI (`tui`, #974).
echo "::group::CLI"
LIBPCAP_LIBDIR=/tmp/libpcap/lib LIBPCAP_VER=$(pin LIBPCAP_VERSION) \
  cargo build --profile dist --locked -p bacnet-cli --features sc-tls,pcap,serial --target "$target"
mkdir -p out
cp "target/$target/dist/bacnet" "out/$cli"
echo "::endgroup::"
ls -l dist out
