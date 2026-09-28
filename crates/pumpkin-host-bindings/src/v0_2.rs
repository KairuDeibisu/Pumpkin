wasmtime::component::bindgen!({
    path: "../pumpkin-plugin-wit/v0.2",
    world: "plugin",
    imports: { default: async | trappable },
    exports: { default: async | store | trappable },
});
