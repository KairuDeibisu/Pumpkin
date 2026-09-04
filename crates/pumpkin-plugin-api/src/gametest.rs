//! Function-based GameTest API for WASM plugins.
//!
//! Plugins register structures and function-backed test instances synchronously. The
//! server invokes the registered Rust function once per GameTest tick; [`Test`] exposes
//! the current tick and host-backed assertions without requiring async plugin code.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{Component, wit};

/// An entity handle scoped to the currently running GameTest invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entity {
    id: i32,
}

/// A running function-based GameTest.
#[derive(Clone, Debug)]
pub struct Test {
    succeeded: Arc<AtomicBool>,
    tick: u32,
}

impl Test {
    fn new(succeeded: Arc<AtomicBool>, tick: u32) -> Self {
        Self { succeeded, tick }
    }

    /// Returns the current GameTest tick, starting at zero when the test body begins.
    #[must_use]
    pub const fn tick(&self) -> u32 {
        self.tick
    }

    /// Marks this GameTest invocation as successful.
    pub fn succeed(&self) {
        self.succeeded.store(true, Ordering::Release);
    }

    /// Evaluates `condition` on and after `start_tick` until it succeeds.
    ///
    /// Assertion errors intentionally leave the GameTest running so the same callback
    /// can be evaluated again on the next tick, matching Bedrock-style `succeedWhen`
    /// behavior while Pumpkin keeps Java Edition setup/max-tick semantics.
    pub fn succeed_when<F>(&self, start_tick: u32, condition: F)
    where
        F: FnOnce(&Test) -> Result<(), String>,
    {
        if self.tick >= start_tick && condition(self).is_ok() {
            self.succeed();
        }
    }

    /// Returns entities of `entity_type` inside the active structure bounds.
    ///
    /// An empty result is treated as a failed assertion, which makes this convenient
    /// to use with `?` inside [`succeed_when`](Self::succeed_when).
    pub fn get_entity_present_in_area(&self, entity_type: &str) -> Result<Vec<Entity>, String> {
        let entities = wit::pumpkin::plugin::gametest::get_entities_in_area(entity_type)?;
        if entities.is_empty() {
            return Err(format!(
                "Expected entity '{entity_type}' inside the GameTest area"
            ));
        }

        Ok(entities
            .into_iter()
            .map(|entity| Entity { id: entity.id })
            .collect())
    }

    /// Asserts that `entity` currently has a passenger of `passenger_type`.
    pub fn assert_entity_has_passenger(
        &self,
        entity: &Entity,
        passenger_type: &str,
    ) -> Result<(), String> {
        wit::pumpkin::plugin::gametest::assert_entity_has_passenger(entity.id, passenger_type)
    }
}

type TestFunction = Arc<dyn Fn(Test) + Send + Sync + 'static>;

struct TestFunctionHandlers {
    handlers: BTreeMap<u32, TestFunction>,
    next_id: u32,
}

static TEST_FUNCTION_HANDLERS: Mutex<TestFunctionHandlers> = Mutex::new(TestFunctionHandlers {
    handlers: BTreeMap::new(),
    next_id: 0,
});

/// Builder for a plugin-defined function GameTest instance.
pub struct TestBuilder {
    id: String,
    handler: TestFunction,
    structure_name: String,
    max_ticks: u32,
    setup_ticks: u32,
    required: bool,
}

impl TestBuilder {
    /// Sets the Java Edition structure resource location used by this test.
    #[must_use]
    pub fn structure_name(mut self, structure_name: &str) -> Self {
        self.structure_name = structure_name.to_string();
        self
    }

    /// Sets the maximum number of running ticks before the test times out.
    #[must_use]
    pub const fn max_ticks(mut self, max_ticks: u32) -> Self {
        self.max_ticks = max_ticks;
        self
    }

    /// Sets the setup delay before the test function begins running.
    #[must_use]
    pub const fn setup_ticks(mut self, setup_ticks: u32) -> Self {
        self.setup_ticks = setup_ticks;
        self
    }

    /// Sets whether failure of this test makes the batch fail.
    #[must_use]
    pub const fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Registers the callback and test instance with Pumpkin.
    pub fn register(self) -> Result<(), String> {
        if self.max_ticks == 0 {
            return Err("GameTest max_ticks must be greater than zero".to_string());
        }

        let handler_id = insert_handler(self.handler);
        if let Err(error) =
            wit::pumpkin::plugin::gametest::register_test_function(&self.id, handler_id)
        {
            remove_handler(handler_id);
            return Err(error);
        }

        let definition = wit::pumpkin::plugin::gametest::GameTest {
            function_id: self.id.clone(),
            structure_name: self.structure_name,
            max_ticks: self.max_ticks,
            setup_ticks: self.setup_ticks,
            required: self.required,
        };

        if let Err(error) = wit::pumpkin::plugin::gametest::register_test(&self.id, &definition) {
            remove_handler(handler_id);
            return Err(error);
        }

        Ok(())
    }
}

/// Starts registration of a function-based GameTest instance.
///
/// Resource-location validation is performed by the Pumpkin host when the builder is registered.
pub fn register<F>(namespace: &str, name: &str, handler: F) -> Result<TestBuilder, String>
where
    F: Fn(Test) + Send + Sync + 'static,
{
    let id = format!("{namespace}:{name}");

    Ok(TestBuilder {
        id,
        handler: Arc::new(handler),
        structure_name: "minecraft:empty".to_string(),
        max_ticks: 100,
        setup_ticks: 0,
        required: true,
    })
}

/// Registers gzipped Java Edition structure NBT for use by plugin GameTests.
pub fn register_structure(id: &str, nbt: &[u8]) -> Result<(), String> {
    if nbt.is_empty() {
        return Err("GameTest structure NBT cannot be empty".to_string());
    }
    wit::pumpkin::plugin::gametest::register_structure(id, nbt)
}

/// Registers a function-based GameTest callback under an existing function id.
///
/// This lower-level API is retained for callers that provide their test-instance
/// definition through a datapack instead of [`register`].
pub fn register_test_function<F>(id: &str, handler: F) -> Result<(), String>
where
    F: Fn(Test) + Send + Sync + 'static,
{
    let handler_id = insert_handler(Arc::new(handler));

    if let Err(error) = wit::pumpkin::plugin::gametest::register_test_function(id, handler_id) {
        remove_handler(handler_id);
        return Err(error);
    }

    Ok(())
}

fn insert_handler(handler: TestFunction) -> u32 {
    let mut handlers = TEST_FUNCTION_HANDLERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let handler_id = handlers.next_id;
    handlers.next_id = handlers.next_id.wrapping_add(1);
    handlers.handlers.insert(handler_id, handler);
    handler_id
}

fn remove_handler(handler_id: u32) {
    TEST_FUNCTION_HANDLERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .handlers
        .remove(&handler_id);
}

impl wit::exports::pumpkin::plugin::gametest_handler::Guest for Component {
    fn handle_test_function(handler_id: u32, tick: u32) -> bool {
        let handler = TEST_FUNCTION_HANDLERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .handlers
            .get(&handler_id)
            .cloned();

        let Some(handler) = handler else {
            return false;
        };

        let succeeded = Arc::new(AtomicBool::new(false));
        handler(Test::new(succeeded.clone(), tick));
        succeeded.load(Ordering::Acquire)
    }
}
