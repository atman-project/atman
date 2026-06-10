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

if ! $BUILD_ARM64 && ! $BUILD_SIM_ARM64 && ! $BUILD_X86_64; then
    "no target specified!"
    exit 1
fi

# UniFFI Swift bindings — `uniffi-bindgen generate --library` introspects
# the compiled scaffolding inside the dylib, so we build a host cdylib
# first. Output is pure Swift source (platform-independent), reused for
# every iOS slice.
SWIFT_OUT="${TARGET_DIR}/uniffi-bindings/swift"
mkdir -p "${SWIFT_OUT}"
cargo build $CARGO_FLAGS $FEATURE_FLAGS -p ${PROJECT_NAME}
cargo run -p atman-uniffi-bindgen --bin uniffi-bindgen -- generate \
    --library "${TARGET_DIR}/${BUILD_MODE}/lib${PROJECT_NAME}.dylib" \
    --language swift \
    --out-dir "${SWIFT_OUT}"

LIB_NAME="lib${PROJECT_NAME}.a"

# Build for ARM64 (iOS devices)
if $BUILD_ARM64; then
    cargo build --target aarch64-apple-ios $CARGO_FLAGS $FEATURE_FLAGS
    lipo -info ${TARGET_DIR}/aarch64-apple-ios/${BUILD_MODE}/${LIB_NAME}
fi

# Build for arm64 iOS simulator (Apple Silicon hosts)
if $BUILD_SIM_ARM64; then
    cargo build --target aarch64-apple-ios-sim $CARGO_FLAGS $FEATURE_FLAGS
    lipo -info ${TARGET_DIR}/aarch64-apple-ios-sim/${BUILD_MODE}/${LIB_NAME}
fi

# Build for x86_64 (Intel iOS simulator)
if $BUILD_X86_64; then
    cargo build --target x86_64-apple-ios $CARGO_FLAGS $FEATURE_FLAGS
    lipo -info ${TARGET_DIR}/x86_64-apple-ios/${BUILD_MODE}/${LIB_NAME}
fi

# Print results
if $BUILD_ARM64; then
    ls -lh ${TARGET_DIR}/aarch64-apple-ios/${BUILD_MODE}/${LIB_NAME}
fi
if $BUILD_SIM_ARM64; then
    ls -lh ${TARGET_DIR}/aarch64-apple-ios-sim/${BUILD_MODE}/${LIB_NAME}
fi
if $BUILD_X86_64; then
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
