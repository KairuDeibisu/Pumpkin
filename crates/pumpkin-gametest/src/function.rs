use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, RwLock},
};

use async_trait::async_trait;

use crate::{GameTestError, GameTestResult};

/// Executable function referenced by a `minecraft:function` GameTest instance.
#[async_trait]
pub trait GameTestFunction: Send + Sync {
    /// Runs the function once when the GameTest enters its running state.
    ///
    /// Returns `true` when the function marked the test successful during this call.
    async fn run(&self) -> GameTestResult<bool>;
}

static TEST_FUNCTIONS: LazyLock<RwLock<HashMap<String, Arc<dyn GameTestFunction>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Registers or replaces a function-based GameTest implementation.
pub fn register_function(id: impl Into<String>, function: Arc<dyn GameTestFunction>) {
    TEST_FUNCTIONS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(id.into(), function);
}

/// Runs a registered function-based GameTest implementation.
pub async fn run_function(id: &str) -> GameTestResult<bool> {
    let function = TEST_FUNCTIONS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(id)
        .cloned()
        .ok_or_else(|| GameTestError::World(format!("Unknown GameTest function '{id}'")))?;

    function.run().await
}
