#!/bin/bash

set -uexo pipefail

PROJECT_NAME="atman"

# Check for --release flag
BUILD_MODE="debug"
if [[ "$*" == *"--release"* ]]; then
    BUILD_MODE="release"
    CARGO_FLAGS="--release"
else
    CARGO_FLAGS=""
fi

cbindgen -l C -o target/${PROJECT_NAME}.h

LIB_NAME="lib${PROJECT_NAME}.a"

cargo build --target aarch64-apple-ios $CARGO_FLAGS
lipo -info target/aarch64-apple-ios/${BUILD_MODE}/${LIB_NAME}

cargo build --target x86_64-apple-ios $CARGO_FLAGS
lipo -info target/x86_64-apple-ios/${BUILD_MODE}/${LIB_NAME}

lipo -create target/aarch64-apple-ios/${BUILD_MODE}/${LIB_NAME} \
    target/x86_64-apple-ios/${BUILD_MODE}/${LIB_NAME} \
    -output target/${BUILD_MODE}/${LIB_NAME}
lipo -info target/${BUILD_MODE}/${LIB_NAME}
