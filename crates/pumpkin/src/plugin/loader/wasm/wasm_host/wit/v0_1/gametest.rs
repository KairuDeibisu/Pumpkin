use std::{
    collections::HashMap,
    sync::{
        Arc, LazyLock, RwLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use wasmtime::component::Resource;

use crate::plugin::loader::wasm::wasm_host::{
    WasmPlugin,
    state::{PluginHostState, WasmResource},
    wit::v0_1::pumpkin::{
        self,
        plugin::gametest::{self, Test as WitTest},
    },
};

/// Minimal shared state for a running plugin-backed GameTest.
///
/// For the initial `minecraft:always_pass` smoke test we only need to know
/// whether the guest called `test.succeed()`.
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

/// A GameTest function supplied by a WASM plugin.
///
/// The datapack owns the test instance. This registry only supplies the
/// executable function referenced by a `minecraft:function` test instance.
#[derive(Clone)]
pub struct RegisteredGameTestFunction {
    pub plugin: Weak<WasmPlugin>,
    pub handler_id: u32,
}

static TEST_FUNCTIONS: LazyLock<RwLock<HashMap<String, RegisteredGameTestFunction>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Looks up a plugin-backed GameTest function.
///
/// The returned `Arc<WasmPlugin>` keeps the plugin alive while the callback is
/// being dispatched.
#[must_use]
pub fn get_test_function(id: &str) -> Option<(Arc<WasmPlugin>, u32)> {
    let registry = TEST_FUNCTIONS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let function = registry.get(id)?.clone();
    let plugin = function.plugin.upgrade()?;

    Some((plugin, function.handler_id))
}

impl PluginHostState {
    /// Adds a running GameTest handle to Wasmtime's resource table.
    ///
    /// The GameTest runner should create one `GameTestControl`, pass the
    /// resulting WIT resource to `handle-test-function`, then inspect
    /// `GameTestControl::has_succeeded()` after the guest callback returns.
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

        TEST_FUNCTIONS
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                id,
                RegisteredGameTestFunction {
                    plugin,
                    handler_id: handler,
                },
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
