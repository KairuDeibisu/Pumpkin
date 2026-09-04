use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, RwLock},
};

use async_trait::async_trait;
use pumpkin_util::math::position::BlockPos;

use crate::{GameTestError, GameTestResult, world::GameTestWorld};

/// Runtime context supplied to a function-based GameTest on each test tick.
#[derive(Clone)]
pub struct GameTestFunctionContext {
    tick: u32,
    world: Arc<dyn GameTestWorld>,
    min: BlockPos,
    max: BlockPos,
}

impl GameTestFunctionContext {
    #[must_use]
    pub(crate) const fn new(
        tick: u32,
        world: Arc<dyn GameTestWorld>,
        min: BlockPos,
        max: BlockPos,
    ) -> Self {
        Self {
            tick,
            world,
            min,
            max,
        }
    }

    #[must_use]
    pub const fn tick(&self) -> u32 {
        self.tick
    }

    pub async fn entities_in_area(&self, entity_type: &str) -> GameTestResult<Vec<i32>> {
        self.world
            .get_entities_in_area(&self.min, &self.max, entity_type)
            .await
    }

    pub async fn assert_entity_has_passenger(
        &self,
        entity_id: i32,
        passenger_type: &str,
    ) -> GameTestResult<()> {
        if self
            .world
            .entity_has_passenger(entity_id, passenger_type)
            .await?
        {
            return Ok(());
        }

        Err(GameTestError::Assertion {
            tick: self.tick,
            position: None,
            message: format!(
                "Expected entity {entity_id} to have passenger '{passenger_type}'"
            ),
        })
    }
}

/// Executable function referenced by a `minecraft:function` GameTest instance.
#[async_trait]
pub trait GameTestFunction: Send + Sync {
    /// Runs the function for the supplied GameTest tick.
    ///
    /// Returns `true` when the function marked the test successful during this call.
    async fn run(&self, context: &GameTestFunctionContext) -> GameTestResult<bool>;
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

/// Runs a registered function-based GameTest implementation for the current tick.
pub async fn run_function(
    id: &str,
    context: &GameTestFunctionContext,
) -> GameTestResult<bool> {
    let function = TEST_FUNCTIONS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(id)
        .cloned()
        .ok_or_else(|| GameTestError::World(format!("Unknown GameTest function '{id}'")))?;

    function.run(context).await
}
