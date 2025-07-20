#!/bin/bash

set -uexo pipefail

PROJECT_NAME="atman"

cbindgen -l C -o target/${PROJECT_NAME}.h

cargo build --target aarch64-apple-ios
lipo -info target/aarch64-apple-ios/debug/lib${PROJECT_NAME}.a

cargo build --target x86_64-apple-ios
lipo -info target/x86_64-apple-ios/debug/lib${PROJECT_NAME}.a

lipo -create target/aarch64-apple-ios/debug/lib${PROJECT_NAME}.a \
    target/x86_64-apple-ios/debug/lib${PROJECT_NAME}.a \
    -output target/lib${PROJECT_NAME}.a
lipo -info target/lib${PROJECT_NAME}.a
