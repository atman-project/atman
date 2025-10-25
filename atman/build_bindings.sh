#!/bin/bash

set -uexo pipefail

PROJECT_NAME="atman"
TARGET_DIR="../target"

# Check for --release flag
BUILD_MODE="debug"
if [[ "$*" == *"--release"* ]]; then
    BUILD_MODE="release"
    CARGO_FLAGS="--release"
else
    CARGO_FLAGS=""
fi

# Architecture flags
BUILD_ARM64=false
BUILD_X86_64=false
if [[ "$*" == *"--arm64"* ]]; then
    BUILD_ARM64=true
fi
if [[ "$*" == *"--x86_64"* ]]; then
    BUILD_X86_64=true
fi
# Default: build both if no specific arch is provided
if ! $BUILD_ARM64 && ! $BUILD_X86_64; then
    BUILD_ARM64=true
    BUILD_X86_64=true
fi

cbindgen -l C -o ${TARGET_DIR}/${PROJECT_NAME}.h

LIB_NAME="lib${PROJECT_NAME}.a"

# Build for ARM64 (iOS devices)
if $BUILD_ARM64; then
    cargo build --target aarch64-apple-ios $CARGO_FLAGS
    lipo -info ${TARGET_DIR}/aarch64-apple-ios/${BUILD_MODE}/${LIB_NAME}
    ls -lh ${TARGET_DIR}/aarch64-apple-ios/${BUILD_MODE}/${LIB_NAME}
fi

# Build for x86_64 (iOS simulator)
if $BUILD_X86_64; then
    cargo build --target x86_64-apple-ios $CARGO_FLAGS
    lipo -info ${TARGET_DIR}/x86_64-apple-ios/${BUILD_MODE}/${LIB_NAME}
    ls -lh ${TARGET_DIR}/x86_64-apple-ios/${BUILD_MODE}/${LIB_NAME}
fi

# Merge the two architectures into a single FAT library
if $BUILD_ARM64 && $BUILD_X86_64; then
    lipo -create ${TARGET_DIR}/aarch64-apple-ios/${BUILD_MODE}/${LIB_NAME} \
        ${TARGET_DIR}/x86_64-apple-ios/${BUILD_MODE}/${LIB_NAME} \
        -output ${TARGET_DIR}/${BUILD_MODE}/${LIB_NAME}
    lipo -info ${TARGET_DIR}/${BUILD_MODE}/${LIB_NAME}
    ls -lh ${TARGET_DIR}/${BUILD_MODE}/${LIB_NAME}
fi
