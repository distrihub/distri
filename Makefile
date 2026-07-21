PROFILE:=release

DEFAULT_CONTAINER_TARGET=x86_64-unknown-linux-gnu
LINUX_ARM_TARGET=aarch64-unknown-linux-gnu
CONTAINER_GLIBC=2.31

MAC_ARM_SLUG=darwin-arm64
MAC_INTEL_SLUG=darwin-amd64
LINUX_X86_SLUG=linux-amd64
LINUX_ARM_SLUG=linux-arm64

UNAME_S:=$(shell uname -s)
ifeq (${UNAME_S},Darwin)
	NPROCS:=$(shell sysctl -n hw.ncpu 2>/dev/null || echo 2)
else
	NPROCS:=$(shell grep -c 'processor' /proc/cpuinfo 2>/dev/null || nproc 2>/dev/null || echo 2)
endif
MAKEFLAGS += -j${NPROCS}

ROOT_DIR=$(shell dirname $(realpath $(firstword $(MAKEFILE_LIST))))
# Release version tracks the CLI crate (produces the `distri` binary and is what
# scripts/publish.env bumps). Must match VERSION_CRATE in scripts/publish.env,
# else release-tarballs writes to the wrong releases/<version>/ dir.
DISTRI_VERSION=$(shell awk -F\" '/^version =/ {print $$2; exit}' ${ROOT_DIR}/distri-cli/Cargo.toml)
RELEASES_DIR=${ROOT_DIR}/releases/${DISTRI_VERSION}
MAC_ARM_TARGET=aarch64-apple-darwin
MAC_INTEL_TARGET=x86_64-apple-darwin
DEPDIR=${TARGETDIR}/deps
TMPDIR=${TARGETDIR}/tmp
RELEASE_TMP=${ROOT_DIR}/target/release-tmp
SYSTEM_TARGET=$(shell rustc -vV | sed -n 's|host: ||p')

ifeq (${PROFILE},dev)
	PROFILE_DIR=debug
else
	PROFILE_DIR=release
endif

TARGETDIR=${ROOT_DIR}/target/${DEFAULT_CONTAINER_TARGET}/${PROFILE_DIR}

ifeq (${SYSTEM_TARGET}, ${DEFAULT_CONTAINER_TARGET})
	RUN_CMD=cargo-zigbuild run --target ${DEFAULT_CONTAINER_TARGET}.${CONTAINER_GLIBC}
else
	RUN_CMD=cargo run
endif

build: build-linux

${TARGETDIR}/distri: ${TMPDIR} FORCE
	cargo zigbuild --profile ${PROFILE} --target ${DEFAULT_CONTAINER_TARGET}.${CONTAINER_GLIBC} -p distri-cli --bin distri 

${TARGETDIR}/distri-server: ${TMPDIR} FORCE
	cargo zigbuild --profile ${PROFILE} --target ${DEFAULT_CONTAINER_TARGET}.${CONTAINER_GLIBC} -p distri-server-cli --bin distri-server --features "sqlite_vendored"

build-all: build-linux build-linux-arm build-mac build-mac-intel

# macOS-only build (used when publishing: Linux targets build on the VM, macOS
# targets build locally since they can't cross-compile from Linux).
build-all-mac: build-mac build-mac-intel

build-linux: ${TMPDIR} FORCE
	cargo zigbuild --profile ${PROFILE} --target ${DEFAULT_CONTAINER_TARGET}.${CONTAINER_GLIBC} -p distri-cli --bin distri
	cargo zigbuild --profile ${PROFILE} --target ${DEFAULT_CONTAINER_TARGET}.${CONTAINER_GLIBC} -p distri-server-cli --bin distri-server --features "sqlite_vendored"

build-linux-arm: ${TMPDIR} FORCE
	cargo zigbuild --profile ${PROFILE} --target ${LINUX_ARM_TARGET}.${CONTAINER_GLIBC} -p distri-cli --bin distri
	cargo zigbuild --profile ${PROFILE} --target ${LINUX_ARM_TARGET}.${CONTAINER_GLIBC} -p distri-server-cli --bin distri-server --features "sqlite_vendored"

build-mac: ${TMPDIR} FORCE
	cargo build --profile ${PROFILE} --target ${MAC_ARM_TARGET} -p distri-cli --bin distri
	cargo build --profile ${PROFILE} --target ${MAC_ARM_TARGET} -p distri-server-cli --bin distri-server --features "sqlite"

build-mac-intel: ${TMPDIR} FORCE
	cargo build --profile ${PROFILE} --target ${MAC_INTEL_TARGET} -p distri-cli --bin distri
	cargo build --profile ${PROFILE} --target ${MAC_INTEL_TARGET} -p distri-server-cli --bin distri-server --features "sqlite"

${TMPDIR}:
	mkdir -p ${TMPDIR}

release-dir:
	mkdir -p ${RELEASES_DIR}

package-releases: build-linux build-linux-arm build-mac build-mac-intel release-tarballs

release-tarballs: release-dir
	@echo "Packaging release tarballs..."
	@# macOS ARM
	@if [ -f "${ROOT_DIR}/target/${MAC_ARM_TARGET}/release/distri" ]; then \
		mkdir -p ${RELEASE_TMP}/${MAC_ARM_SLUG} && \
		cp -p ${ROOT_DIR}/LICENSE ${RELEASE_TMP}/${MAC_ARM_SLUG}/LICENSE && \
		cp -p ${ROOT_DIR}/target/${MAC_ARM_TARGET}/release/distri ${RELEASE_TMP}/${MAC_ARM_SLUG}/distri; \
		if [ -f "${ROOT_DIR}/target/${MAC_ARM_TARGET}/release/distri-server" ]; then \
			mkdir -p ${RELEASE_TMP}/${MAC_ARM_SLUG}/server && \
			cp -p ${ROOT_DIR}/server/LICENSE ${RELEASE_TMP}/${MAC_ARM_SLUG}/server/LICENSE && \
			cp -p ${ROOT_DIR}/target/${MAC_ARM_TARGET}/release/distri-server ${RELEASE_TMP}/${MAC_ARM_SLUG}/server/distri-server; \
		fi && \
		tar -czf ${RELEASES_DIR}/distri-${MAC_ARM_SLUG}.tar.gz -C ${RELEASE_TMP} ${MAC_ARM_SLUG}; \
	fi
	@# macOS Intel
	@if [ -f "${ROOT_DIR}/target/${MAC_INTEL_TARGET}/release/distri" ]; then \
		mkdir -p ${RELEASE_TMP}/${MAC_INTEL_SLUG} && \
		cp -p ${ROOT_DIR}/LICENSE ${RELEASE_TMP}/${MAC_INTEL_SLUG}/LICENSE && \
		cp -p ${ROOT_DIR}/target/${MAC_INTEL_TARGET}/release/distri ${RELEASE_TMP}/${MAC_INTEL_SLUG}/distri; \
		if [ -f "${ROOT_DIR}/target/${MAC_INTEL_TARGET}/release/distri-server" ]; then \
			mkdir -p ${RELEASE_TMP}/${MAC_INTEL_SLUG}/server && \
			cp -p ${ROOT_DIR}/server/LICENSE ${RELEASE_TMP}/${MAC_INTEL_SLUG}/server/LICENSE && \
			cp -p ${ROOT_DIR}/target/${MAC_INTEL_TARGET}/release/distri-server ${RELEASE_TMP}/${MAC_INTEL_SLUG}/server/distri-server; \
		fi && \
		tar -czf ${RELEASES_DIR}/distri-${MAC_INTEL_SLUG}.tar.gz -C ${RELEASE_TMP} ${MAC_INTEL_SLUG}; \
	fi
	@# Linux x86_64
	mkdir -p ${RELEASE_TMP}/${LINUX_X86_SLUG}
	cp -p ${ROOT_DIR}/LICENSE ${RELEASE_TMP}/${LINUX_X86_SLUG}/LICENSE
	cp -p ${ROOT_DIR}/target/${DEFAULT_CONTAINER_TARGET}/release/distri ${RELEASE_TMP}/${LINUX_X86_SLUG}/distri
	@if [ -f "${ROOT_DIR}/target/${DEFAULT_CONTAINER_TARGET}/release/distri-server" ]; then \
		mkdir -p ${RELEASE_TMP}/${LINUX_X86_SLUG}/server && \
		cp -p ${ROOT_DIR}/server/LICENSE ${RELEASE_TMP}/${LINUX_X86_SLUG}/server/LICENSE && \
		cp -p ${ROOT_DIR}/target/${DEFAULT_CONTAINER_TARGET}/release/distri-server ${RELEASE_TMP}/${LINUX_X86_SLUG}/server/distri-server; \
	fi
	tar -czf ${RELEASES_DIR}/distri-${LINUX_X86_SLUG}.tar.gz -C ${RELEASE_TMP} ${LINUX_X86_SLUG}
	@# Linux ARM
	mkdir -p ${RELEASE_TMP}/${LINUX_ARM_SLUG}
	cp -p ${ROOT_DIR}/LICENSE ${RELEASE_TMP}/${LINUX_ARM_SLUG}/LICENSE
	cp -p ${ROOT_DIR}/target/${LINUX_ARM_TARGET}/release/distri ${RELEASE_TMP}/${LINUX_ARM_SLUG}/distri
	@if [ -f "${ROOT_DIR}/target/${LINUX_ARM_TARGET}/release/distri-server" ]; then \
		mkdir -p ${RELEASE_TMP}/${LINUX_ARM_SLUG}/server && \
		cp -p ${ROOT_DIR}/server/LICENSE ${RELEASE_TMP}/${LINUX_ARM_SLUG}/server/LICENSE && \
		cp -p ${ROOT_DIR}/target/${LINUX_ARM_TARGET}/release/distri-server ${RELEASE_TMP}/${LINUX_ARM_SLUG}/server/distri-server; \
	fi
	tar -czf ${RELEASES_DIR}/distri-${LINUX_ARM_SLUG}.tar.gz -C ${RELEASE_TMP} ${LINUX_ARM_SLUG}

release-tarballs-mac: release-dir
	@echo "Packaging macOS release tarballs..."
	@# macOS ARM
	@if [ -f "${ROOT_DIR}/target/${MAC_ARM_TARGET}/release/distri" ]; then \
		mkdir -p ${RELEASE_TMP}/${MAC_ARM_SLUG} && \
		cp -p ${ROOT_DIR}/LICENSE ${RELEASE_TMP}/${MAC_ARM_SLUG}/LICENSE && \
		cp -p ${ROOT_DIR}/target/${MAC_ARM_TARGET}/release/distri ${RELEASE_TMP}/${MAC_ARM_SLUG}/distri; \
		if [ -f "${ROOT_DIR}/target/${MAC_ARM_TARGET}/release/distri-server" ]; then \
			mkdir -p ${RELEASE_TMP}/${MAC_ARM_SLUG}/server && \
			cp -p ${ROOT_DIR}/server/LICENSE ${RELEASE_TMP}/${MAC_ARM_SLUG}/server/LICENSE && \
			cp -p ${ROOT_DIR}/target/${MAC_ARM_TARGET}/release/distri-server ${RELEASE_TMP}/${MAC_ARM_SLUG}/server/distri-server; \
		fi && \
		tar -czf ${RELEASES_DIR}/distri-${MAC_ARM_SLUG}.tar.gz -C ${RELEASE_TMP} ${MAC_ARM_SLUG}; \
	fi
	@# macOS Intel
	@if [ -f "${ROOT_DIR}/target/${MAC_INTEL_TARGET}/release/distri" ]; then \
		mkdir -p ${RELEASE_TMP}/${MAC_INTEL_SLUG} && \
		cp -p ${ROOT_DIR}/LICENSE ${RELEASE_TMP}/${MAC_INTEL_SLUG}/LICENSE && \
		cp -p ${ROOT_DIR}/target/${MAC_INTEL_TARGET}/release/distri ${RELEASE_TMP}/${MAC_INTEL_SLUG}/distri; \
		if [ -f "${ROOT_DIR}/target/${MAC_INTEL_TARGET}/release/distri-server" ]; then \
			mkdir -p ${RELEASE_TMP}/${MAC_INTEL_SLUG}/server && \
			cp -p ${ROOT_DIR}/server/LICENSE ${RELEASE_TMP}/${MAC_INTEL_SLUG}/server/LICENSE && \
			cp -p ${ROOT_DIR}/target/${MAC_INTEL_TARGET}/release/distri-server ${RELEASE_TMP}/${MAC_INTEL_SLUG}/server/distri-server; \
		fi && \
		tar -czf ${RELEASES_DIR}/distri-${MAC_INTEL_SLUG}.tar.gz -C ${RELEASE_TMP} ${MAC_INTEL_SLUG}; \
	fi

FORCE: ;
