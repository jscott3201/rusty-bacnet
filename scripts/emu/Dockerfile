# QEMU for scripts/emu/emulate-firmware.sh: the firmware link test's images
# run on emulated Cortex-M and RISC-V boards (docs/ci.md, "Emulation
# (manual)"). The script builds this image on demand and tags it with this
# file's SHA-256, so any edit here gives a new tag; it never pulls a tag.
#
# Reproducible: Debian 13.7 (trixie) slim pinned by its index digest (amd64
# and arm64), apt pointed at the Debian snapshot archive as of the date the
# base image was built from (its debian.sources records it), and both QEMU
# packages pinned to that snapshot's version. apt checks each Release file
# against the Debian archive keyring, so plain HTTP is fine, and old
# snapshots' Valid-Until dates are ignored. snapshot.debian.org throttles
# busy clients, so each fetch is retried up to 5 times. trixie packages the
# RISC-V emulators apart from qemu-system-misc, which keeps the image near
# 330 MB.
#
# To move to a newer QEMU: take a newer trixie-slim digest, its snapshot
# date from /etc/apt/sources.list.d/debian.sources, and the version
# `apt-cache policy qemu-system-arm` shows there; then run the script, which
# builds the new image, and record the version in docs/ci.md.
FROM debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f

ARG SNAPSHOT=20261005T000000Z
ARG QEMU_VERSION=1:10.0.13+ds-0+deb13u1

RUN printf '%s\n' \
      'Types: deb' \
      "URIs: http://snapshot.debian.org/archive/debian/${SNAPSHOT}" \
      'Suites: trixie trixie-updates' \
      'Components: main' \
      'Signed-By: /usr/share/keyrings/debian-archive-keyring.pgp' \
      '' \
      'Types: deb' \
      "URIs: http://snapshot.debian.org/archive/debian-security/${SNAPSHOT}" \
      'Suites: trixie-security' \
      'Components: main' \
      'Signed-By: /usr/share/keyrings/debian-archive-keyring.pgp' \
      > /etc/apt/sources.list.d/debian.sources \
    && apt-get -o Acquire::Retries=5 -o Acquire::Check-Valid-Until=false update \
    && apt-get -o Acquire::Retries=5 install -y --no-install-recommends \
      "qemu-system-arm=${QEMU_VERSION}" \
      "qemu-system-riscv=${QEMU_VERSION}" \
    && rm -rf /var/lib/apt/lists/*

# emulate-firmware.sh mounts scripts/emu here, and the images elsewhere, both
# read-only, and runs run-qemu.sh from here.
WORKDIR /emu
