PROFILE:=release

DEFAULT_CONTAINER_TARGET=x86_64-unknown-linux-gnu
CONTAINER_GLIBC=2.31

ROOT_DIR=$(shell dirname $(realpath $(firstword $(MAKEFILE_LIST))))
DEPDIR=${TARGETDIR}/deps
TMPDIR=${TARGETDIR}/tmp
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

build: ${TARGETDIR}/distri

${TARGETDIR}/distri: ${TMPDIR} FORCE
	cargo zigbuild --profile ${PROFILE} --target ${DEFAULT_CONTAINER_TARGET}.${CONTAINER_GLIBC} --bin distri

${TMPDIR}:
	mkdir -p ${TMPDIR}

FORCE: ;

