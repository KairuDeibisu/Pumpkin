use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, RwLock, Weak},
};

use async_trait::async_trait;
use pumpkin_gametest::{
    GameTestDefinition, GameTestError, GameTestFunction, GameTestResult, GameTestRotation, TestType,
    function::GameTestFunctionContext, register_function,
};
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
        if let Err(error) = validate_resource_location(&id) {
            return Ok(Err(error));
        }

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
            id,
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
        if let Err(error) = validate_resource_location(&id) {
            return Ok(Err(error));
        }
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
        if let Err(error) = validate_resource_location(&id) {
            return Ok(Err(error));
        }
        if let Err(error) = validate_resource_location(&test.function_id) {
            return Ok(Err(format!("Invalid GameTest function id: {error}")));
        }
        if let Err(error) = validate_resource_location(&test.structure_name) {
            return Ok(Err(format!("Invalid GameTest structure id: {error}")));
        }
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
            structure: test.structure_name,
            function: Some(test.function_id),
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
            .register_plugin_test_instance(&id, definition))
    }

    async fn get_entities_in_area(
        &mut self,
        entity_type: String,
    ) -> wasmtime::Result<Result<Vec<gametest::TestEntity>, String>> {
        if let Err(error) = validate_resource_location(&entity_type) {
            return Ok(Err(error));
        }
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
        if let Err(error) = validate_resource_location(&passenger_type) {
            return Ok(Err(error));
        }
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

fn validate_resource_location(value: &str) -> Result<(), String> {
    let Some((namespace, path)) = value.split_once(':') else {
        return Err(format!("'{value}' is not a namespaced resource location"));
    };
    let valid_namespace = !namespace.is_empty()
        && namespace
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte));
    let valid_path = !path.is_empty()
        && !path.contains(':')
        && !path.split('/').any(|segment| segment == "." || segment == "..")
        && path.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || b"_./-".contains(&byte)
        });
    if !valid_namespace || !valid_path {
        return Err(format!("Invalid resource location '{value}'"));
    }
    Ok(())
}
