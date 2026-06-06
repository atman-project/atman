#!/bin/bash

RUST_LOG=iroh=info,atman=info \
	cargo run --bin atman -- \
	--syncman-dir $TMPDIR/t10 \
	--identity a2b44b8043368cb06d6180f2318fa0c87c3869a66d3999600e4e4c325a4b56c1 \
	--network-key e6b5f2694334c26a7f02062b99ab7735f4acc97c017502e0d7490331540ab1bc \
	--rest-addr 127.0.0.1:8888 \
	--sync-interval 3s \
	daemonize
