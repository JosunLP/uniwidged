//! The generated component bindings.
//!
//! Only on `wasm32`: the generated code links against the component model's
//! imports, which do not exist anywhere else. Gating it here means a widget
//! author can `cargo test` their own logic natively without a WASM toolchain,
//! which is the difference between a framework people try and one they do not.

wit_bindgen::generate!({
    path: "wit",
    world: "widget",
    pub_export_macro: true,
});
