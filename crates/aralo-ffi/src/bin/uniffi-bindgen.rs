//! The binding generator, pinned to the same UniFFI version as the library by
//! living in the same crate. Built only with `--features bindgen`.

fn main() {
    uniffi::uniffi_bindgen_main()
}
