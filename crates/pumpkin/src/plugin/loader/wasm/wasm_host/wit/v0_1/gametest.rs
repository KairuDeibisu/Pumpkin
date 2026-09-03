use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use pumpkin_gametest::{
    GameTestError, GameTestFunction, GameTestResult, register_function,
};
use wasmtime::component::Resource;

use crate::plugin::loader::wasm::wasm_host::{
    PluginInstance, WasmPlugin,
    state::{PluginHostState, WasmResource},
    wit::v0_1::pumpkin::plugin::gametest::{self, Test as WitTest},
};

/// Minimal shared state for a running plugin-backed GameTest.
///
/// The first function-based API only needs to know whether the guest called
/// `test.succeed()` during the test-function callback.
#[derive(Debug, Default)]
pub struct GameTestControl {
    succeeded: AtomicBool,
}

impl GameTestControl {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            succeeded: AtomicBool::new(false),
        }
    }

    pub fn succeed(&self) {
        self.succeeded.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn has_succeeded(&self) -> bool {
        self.succeeded.load(Ordering::Acquire)
    }
}

type GameTestResource = WasmResource<Arc<GameTestControl>>;

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
        let control = Arc::new(GameTestControl::new());
        let mut store = plugin.store.lock().await;
        let test = store
            .data_mut()
            .add_game_test(control.clone())
            .map_err(|error| GameTestError::World(error.to_string()))?;

        match &plugin.plugin_instance {
            PluginInstance::V0_1(plugin_instance) => plugin_instance
                .pumpkin_plugin_gametest_handler()
                .call_handle_test_function(&mut *store, self.handler_id, test)
                .await
                .map_err(|error| GameTestError::World(error.to_string()))?,
        }

        Ok(control.has_succeeded())
    }
}

impl PluginHostState {
    /// Adds a running GameTest handle to Wasmtime's resource table.
    pub fn add_game_test(
        &mut self,
        control: Arc<GameTestControl>,
    ) -> wasmtime::Result<Resource<WitTest>> {
        let resource = self
            .resource_table
            .push(GameTestResource { provider: control })?;
        Ok(Resource::new_own(resource.rep()))
    }

    fn get_game_test(&self, resource: &Resource<WitTest>) -> wasmtime::Result<&GameTestResource> {
        self.resource_table
            .get::<GameTestResource>(&Resource::new_own(resource.rep()))
            .map_err(wasmtime::Error::from)
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

impl gametest::HostTest for PluginHostState {
    async fn succeed(&mut self, test: Resource<WitTest>) -> wasmtime::Result<()> {
        self.get_game_test(&test)?.provider.succeed();
        Ok(())
    }

    async fn drop(&mut self, test: Resource<WitTest>) -> wasmtime::Result<()> {
        self.resource_table
            .delete::<GameTestResource>(Resource::new_own(test.rep()))
            .map_err(wasmtime::Error::from)?;
        Ok(())
    }
}
