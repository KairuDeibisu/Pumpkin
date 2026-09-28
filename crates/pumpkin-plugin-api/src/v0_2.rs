//! WIT 0.2 bindings for plugin-supplied GameTest functions.
#![allow(missing_docs)]
wit_bindgen::generate!({
    path: "../pumpkin-plugin-wit/v0.2",
    world: "plugin",
    generate_all,
});
