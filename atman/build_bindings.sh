#!/bin/bash

set -uexo pipefail

PROJECT_NAME="atman"
TARGET_DIR="../target"

BUILD_MODE="debug"
BUILD_ARM64=false
BUILD_SIM_ARM64=false
BUILD_X86_64=false
FEATURE_FLAGS=""

while (( $# )); do
    case "$1" in
        --release)
            BUILD_MODE="release"
            ;;
        --arm64)
            BUILD_ARM64=true
            ;;
        --sim-arm64)
            BUILD_SIM_ARM64=true
            ;;
        --x86_64)
            BUILD_X86_64=true
            ;;
        --features)
            if [[ -n "$FEATURE_FLAGS" ]]; then
                echo "build_bindings.sh: --features and --all-features are mutually exclusive" >&2
                exit 2
            fi
            FEATURE_FLAGS="--features $2"
            shift
            ;;
        --all-features)
            if [[ -n "$FEATURE_FLAGS" ]]; then
                echo "build_bindings.sh: --features and --all-features are mutually exclusive" >&2
                exit 2
            fi
            FEATURE_FLAGS="--all-features"
            ;;
        *)
            echo "build_bindings.sh: unknown argument: $1" >&2
            exit 2
            ;;
    esac
    shift
done

CARGO_FLAGS=""
if [[ "$BUILD_MODE" == "release" ]]; then
    CARGO_FLAGS="--release"
fi

# Default: build arm64 if no target was specified.
if ! $BUILD_ARM64 && ! $BUILD_SIM_ARM64 && ! $BUILD_X86_64; then
    BUILD_ARM64=true
fi

cbindgen -l C -o ${TARGET_DIR}/${PROJECT_NAME}.h

LIB_NAME="lib${PROJECT_NAME}.a"

# Build for ARM64 (iOS devices)
if $BUILD_ARM64; then
    cargo build --target aarch64-apple-ios $CARGO_FLAGS $FEATURE_FLAGS
    lipo -info ${TARGET_DIR}/aarch64-apple-ios/${BUILD_MODE}/${LIB_NAME}
    ls -lh ${TARGET_DIR}/aarch64-apple-ios/${BUILD_MODE}/${LIB_NAME}
fi

# Build for arm64 iOS simulator (Apple Silicon hosts)
if $BUILD_SIM_ARM64; then
    cargo build --target aarch64-apple-ios-sim $CARGO_FLAGS $FEATURE_FLAGS
    lipo -info ${TARGET_DIR}/aarch64-apple-ios-sim/${BUILD_MODE}/${LIB_NAME}
    ls -lh ${TARGET_DIR}/aarch64-apple-ios-sim/${BUILD_MODE}/${LIB_NAME}
fi

# Build for x86_64 (Intel iOS simulator)
if $BUILD_X86_64; then
    cargo build --target x86_64-apple-ios $CARGO_FLAGS $FEATURE_FLAGS
    lipo -info ${TARGET_DIR}/x86_64-apple-ios/${BUILD_MODE}/${LIB_NAME}
    ls -lh ${TARGET_DIR}/x86_64-apple-ios/${BUILD_MODE}/${LIB_NAME}
fi

# Merge the two architectures into a single FAT library
#if $BUILD_ARM64 && $BUILD_X86_64; then
#    lipo -create ${TARGET_DIR}/aarch64-apple-ios/${BUILD_MODE}/${LIB_NAME} \
#        ${TARGET_DIR}/x86_64-apple-ios/${BUILD_MODE}/${LIB_NAME} \
#        -output ${TARGET_DIR}/${BUILD_MODE}/${LIB_NAME}
#    lipo -info ${TARGET_DIR}/${BUILD_MODE}/${LIB_NAME}
#    ls -lh ${TARGET_DIR}/${BUILD_MODE}/${LIB_NAME}
#fi
