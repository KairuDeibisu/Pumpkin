use std::sync::{Arc, Weak};

use async_trait::async_trait;
use pumpkin_gametest::{GameTestError, GameTestFunction, GameTestResult, register_function};

use crate::plugin::loader::wasm::wasm_host::{
    PluginInstance, WasmPlugin,
    state::PluginHostState,
    wit::v0_1::pumpkin::plugin::gametest,
};

struct WasmGameTestFunction {
    plugin: Weak<WasmPlugin>,
    handler_id: u32,
}

#[async_trait]
impl GameTestFunction for WasmGameTestFunction {
    async fn run(&self) -> GameTestResult<bool> {
        let plugin = self.plugin.upgrade().ok_or_else(|| {
            GameTestError::World("GameTest function belongs to an unloaded plugin".to_string())
        })?;
        let mut store = plugin.store.lock().await;

        match &plugin.plugin_instance {
            PluginInstance::V0_1(plugin_instance) => plugin_instance
                .pumpkin_plugin_gametest_handler()
                .call_handle_test_function(&mut *store, self.handler_id)
                .await
                .map_err(|error| GameTestError::World(error.to_string())),
        }
    }
}

impl gametest::Host for PluginHostState {
    async fn register_test_function(
        &mut self,
        id: String,
        handler: u32,
    ) -> wasmtime::Result<Result<(), String>> {
        if id.is_empty() {
            return Ok(Err("GameTest function id cannot be empty".to_string()));
        }

        let plugin = self
            .plugin
            .as_ref()
            .cloned()
            .ok_or_else(|| wasmtime::Error::msg("Plugin not found"))?;

        register_function(
            id,
            Arc::new(WasmGameTestFunction {
                plugin,
                handler_id: handler,
            }),
        );

        Ok(Ok(()))
    }
}
