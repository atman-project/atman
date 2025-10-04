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

cbindgen -l C -o ${TARGET_DIR}/${PROJECT_NAME}.h

LIB_NAME="lib${PROJECT_NAME}.a"

cargo build --target aarch64-apple-ios $CARGO_FLAGS
lipo -info ${TARGET_DIR}/aarch64-apple-ios/${BUILD_MODE}/${LIB_NAME}

cargo build --target x86_64-apple-ios $CARGO_FLAGS
lipo -info ${TARGET_DIR}/x86_64-apple-ios/${BUILD_MODE}/${LIB_NAME}

lipo -create ${TARGET_DIR}/aarch64-apple-ios/${BUILD_MODE}/${LIB_NAME} \
    ${TARGET_DIR}/x86_64-apple-ios/${BUILD_MODE}/${LIB_NAME} \
    -output ${TARGET_DIR}/${BUILD_MODE}/${LIB_NAME}
lipo -info ${TARGET_DIR}/${BUILD_MODE}/${LIB_NAME}
