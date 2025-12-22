//! uniffi-bindgen binary for generating language bindings.
//!
//! Usage:
//! ```bash
//! cargo run -p flovyn-ffi --bin uniffi-bindgen -- \
//!     generate --library target/debug/libflovyn_ffi.dylib \
//!     --language kotlin --out-dir out/kotlin
//! ```

fn main() {
    uniffi::uniffi_bindgen_main()
}
