use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, RwLock, Weak},
};

use async_trait::async_trait;
use pumpkin_gametest::{
    GameTestDefinition, GameTestError, GameTestFunction, GameTestResult, GameTestRotation, TestType,
    function::GameTestFunctionContext, register_function,
};
use pumpkin_util::identifier::Identifier;
use serde_json::Value;

use crate::plugin::loader::wasm::wasm_host::{
    PluginInstance, WasmPlugin,
    state::PluginHostState,
    wit::v0_1::pumpkin::plugin::gametest,
};

static ACTIVE_CONTEXTS: LazyLock<RwLock<HashMap<String, GameTestFunctionContext>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

struct WasmGameTestFunction {
    plugin: Weak<WasmPlugin>,
    plugin_name: String,
    handler_id: u32,
}

#[async_trait]
impl GameTestFunction for WasmGameTestFunction {
    async fn run(&self, context: &GameTestFunctionContext) -> GameTestResult<bool> {
        let plugin = self.plugin.upgrade().ok_or_else(|| {
            GameTestError::World("GameTest function belongs to an unloaded plugin".to_string())
        })?;
        let mut store = plugin.store.lock().await;

        ACTIVE_CONTEXTS
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(self.plugin_name.clone(), context.clone());

        let result = match &plugin.plugin_instance {
            PluginInstance::V0_1(plugin_instance) => plugin_instance
                .pumpkin_plugin_gametest_handler()
                .call_handle_test_function(&mut *store, self.handler_id, context.tick())
                .await
                .map_err(|error| GameTestError::World(error.to_string())),
        };

        ACTIVE_CONTEXTS
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.plugin_name);

        result
    }
}

impl gametest::Host for PluginHostState {
    async fn register_test_function(
        &mut self,
        id: String,
        handler: u32,
    ) -> wasmtime::Result<Result<(), String>> {
        let id = match parse_identifier(&id) {
            Ok(id) => id,
            Err(error) => return Ok(Err(error)),
        };

        let plugin = self
            .plugin
            .as_ref()
            .cloned()
            .ok_or_else(|| wasmtime::Error::msg("Plugin not found"))?;
        let plugin_name = self
            .name
            .clone()
            .ok_or_else(|| wasmtime::Error::msg("Plugin name not initialized"))?;

        register_function(
            id.to_string(),
            Arc::new(WasmGameTestFunction {
                plugin,
                plugin_name,
                handler_id: handler,
            }),
        );

        Ok(Ok(()))
    }

    async fn register_structure(
        &mut self,
        id: String,
        nbt: Vec<u8>,
    ) -> wasmtime::Result<Result<(), String>> {
        let id = match parse_identifier(&id) {
            Ok(id) => id,
            Err(error) => return Ok(Err(error)),
        };
        if nbt.is_empty() {
            return Ok(Err("GameTest structure NBT cannot be empty".to_string()));
        }

        let server = self
            .server
            .as_ref()
            .ok_or_else(|| wasmtime::Error::msg("Server not initialized"))?;
        let plugin_name = self
            .name
            .as_deref()
            .ok_or_else(|| wasmtime::Error::msg("Plugin name not initialized"))?;
        let world_path = server.basic_config.get_world_path();

        Ok(server.datapack_manager.register_plugin_test_structure(
            &world_path,
            plugin_name,
            &id,
            &nbt,
        ))
    }

    async fn register_test(
        &mut self,
        id: String,
        test: gametest::GameTest,
    ) -> wasmtime::Result<Result<(), String>> {
        let id = match parse_identifier(&id) {
            Ok(id) => id,
            Err(error) => return Ok(Err(error)),
        };
        let function_id = match parse_identifier(&test.function_id) {
            Ok(id) => id,
            Err(error) => return Ok(Err(format!("Invalid GameTest function id: {error}"))),
        };
        let structure_name = match parse_identifier(&test.structure_name) {
            Ok(id) => id,
            Err(error) => return Ok(Err(format!("Invalid GameTest structure id: {error}"))),
        };
        if test.max_ticks == 0 {
            return Ok(Err("GameTest max_ticks must be greater than zero".to_string()));
        }

        let max_ticks = match i32::try_from(test.max_ticks) {
            Ok(value) => value,
            Err(_) => return Ok(Err("GameTest max_ticks exceeds i32::MAX".to_string())),
        };
        let setup_ticks = match i32::try_from(test.setup_ticks) {
            Ok(value) => value,
            Err(_) => return Ok(Err("GameTest setup_ticks exceeds i32::MAX".to_string())),
        };

        let server = self
            .server
            .as_ref()
            .ok_or_else(|| wasmtime::Error::msg("Server not initialized"))?;
        let definition = GameTestDefinition {
            instance_type: TestType::Function,
            environment: Value::String("minecraft:default".to_string()),
            structure: structure_name.to_string(),
            function: Some(function_id.to_string()),
            max_ticks,
            setup_ticks,
            required: test.required,
            rotation: GameTestRotation::None,
            manual_only: false,
            max_attempts: 1,
            required_successes: 1,
            sky_access: false,
            padding: 0,
        };

        Ok(server
            .datapack_manager
            .register_plugin_test_instance(id, definition))
    }

    async fn get_entities_in_area(
        &mut self,
        entity_type: String,
    ) -> wasmtime::Result<Result<Vec<gametest::TestEntity>, String>> {
        let entity_type = match parse_identifier(&entity_type) {
            Ok(id) => id.to_string(),
            Err(error) => return Ok(Err(error)),
        };
        let context = match active_context(self) {
            Ok(context) => context,
            Err(error) => return Ok(Err(error)),
        };

        Ok(match context.entities_in_area(&entity_type).await {
            Ok(entities) => Ok(entities
                .into_iter()
                .map(|id| gametest::TestEntity { id })
                .collect()),
            Err(error) => Err(error.to_string()),
        })
    }

    async fn assert_entity_has_passenger(
        &mut self,
        entity_id: i32,
        passenger_type: String,
    ) -> wasmtime::Result<Result<(), String>> {
        let passenger_type = match parse_identifier(&passenger_type) {
            Ok(id) => id.to_string(),
            Err(error) => return Ok(Err(error)),
        };
        let context = match active_context(self) {
            Ok(context) => context,
            Err(error) => return Ok(Err(error)),
        };

        Ok(context
            .assert_entity_has_passenger(entity_id, &passenger_type)
            .await
            .map_err(|error| error.to_string()))
    }
}

fn active_context(state: &PluginHostState) -> Result<GameTestFunctionContext, String> {
    let plugin_name = state
        .name
        .as_deref()
        .ok_or_else(|| "Plugin name not initialized".to_string())?;
    ACTIVE_CONTEXTS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(plugin_name)
        .cloned()
        .ok_or_else(|| "GameTest assertion called outside an active test callback".to_string())
}

fn parse_identifier(value: &str) -> Result<Identifier, String> {
    Identifier::parse(value).map_err(|error| error.to_string())
}
