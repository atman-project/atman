// Standard UniFFI bindgen entry point. The `cli` feature of `uniffi`
// supplies the full CLI; we only need to delegate to it so the binary
// name (`uniffi-bindgen`) matches what build scripts in beam-ios and
// beam-android invoke.
fn main() {
    uniffi::uniffi_bindgen_main()
}
