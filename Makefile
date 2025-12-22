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
DISTRI_VERSION=$(shell awk -F\" '/^version =/ {print $$2; exit}' ${ROOT_DIR}/distri/Cargo.toml)
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
FRONTEND_DIR=${ROOT_DIR}/distrijs/apps/distri-ui
FRONTEND_DIST=${ROOT_DIR}/distrijs/dist

ifeq (${SYSTEM_TARGET}, ${DEFAULT_CONTAINER_TARGET})
	RUN_CMD=cargo-zigbuild run --target ${DEFAULT_CONTAINER_TARGET}.${CONTAINER_GLIBC}
else
	RUN_CMD=cargo run
endif

build: build-linux

${TARGETDIR}/distri: ${TMPDIR} FORCE
	cargo zigbuild --profile ${PROFILE} --target ${DEFAULT_CONTAINER_TARGET}.${CONTAINER_GLIBC} -p distri-cli --bin distri 

build-all: build-linux build-linux-arm build-mac build-mac-intel

build-linux:  ${TMPDIR} FORCE
	cargo zigbuild --profile ${PROFILE} --target ${DEFAULT_CONTAINER_TARGET}.${CONTAINER_GLIBC} -p distri-cli --bin distri 

build-linux-arm: ${TMPDIR} FORCE
	cargo zigbuild --profile ${PROFILE} --target ${LINUX_ARM_TARGET}.${CONTAINER_GLIBC} -p distri-cli --bin distri 

build-mac: ${TMPDIR} FORCE
	cargo build --profile ${PROFILE} -p distri-cli --bin distri 

build-mac-intel:  ${TMPDIR} FORCE
	cargo build --profile ${PROFILE} --target ${MAC_INTEL_TARGET} -p distri-cli --bin distri 


${TMPDIR}:
	mkdir -p ${TMPDIR}

release-dir:
	mkdir -p ${RELEASES_DIR}

package-releases: build-linux build-linux-arm build-mac build-mac-intel release-tarballs

release-tarballs: release-dir
	mkdir -p ${RELEASE_TMP}/${MAC_ARM_SLUG} ${RELEASE_TMP}/${MAC_INTEL_SLUG} ${RELEASE_TMP}/${LINUX_X86_SLUG} ${RELEASE_TMP}/${LINUX_ARM_SLUG}
	cp -p ${ROOT_DIR}/target/release/distri ${RELEASE_TMP}/${MAC_ARM_SLUG}/distri
	tar -czf ${RELEASES_DIR}/distri-${MAC_ARM_SLUG}.tar.gz -C ${RELEASE_TMP}/${MAC_ARM_SLUG} distri
	cp -p ${ROOT_DIR}/target/${MAC_INTEL_TARGET}/release/distri ${RELEASE_TMP}/${MAC_INTEL_SLUG}/distri
	tar -czf ${RELEASES_DIR}/distri-${MAC_INTEL_SLUG}.tar.gz -C ${RELEASE_TMP}/${MAC_INTEL_SLUG} distri
	cp -p ${ROOT_DIR}/target/${DEFAULT_CONTAINER_TARGET}/release/distri ${RELEASE_TMP}/${LINUX_X86_SLUG}/distri
	tar -czf ${RELEASES_DIR}/distri-${LINUX_X86_SLUG}.tar.gz -C ${RELEASE_TMP}/${LINUX_X86_SLUG} distri
	cp -p ${ROOT_DIR}/target/${LINUX_ARM_TARGET}/release/distri ${RELEASE_TMP}/${LINUX_ARM_SLUG}/distri
	tar -czf ${RELEASES_DIR}/distri-${LINUX_ARM_SLUG}.tar.gz -C ${RELEASE_TMP}/${LINUX_ARM_SLUG} distri

FORCE: ;
