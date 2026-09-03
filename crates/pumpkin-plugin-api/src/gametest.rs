//! Function-based GameTest API for WASM plugins.
//!
//! Register a Rust function with [`register_test_function`], then reference its
//! namespaced ID from a `minecraft:function` test instance.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub use crate::wit::pumpkin::plugin::gametest::Test;
use crate::{Component, wit};

type TestFunction = Arc<dyn Fn(Test) + Send + Sync + 'static>;

struct TestFunctionHandlers {
    handlers: BTreeMap<u32, TestFunction>,
    next_id: u32,
}

static TEST_FUNCTION_HANDLERS: Mutex<TestFunctionHandlers> = Mutex::new(TestFunctionHandlers {
    handlers: BTreeMap::new(),
    next_id: 0,
});

/// Registers a function-based GameTest callback.
///
/// `id` is the namespaced test-function ID referenced by the `function` field of
/// a `minecraft:function` test instance.
pub fn register_test_function<F>(id: &str, handler: F) -> Result<(), String>
where
    F: Fn(Test) + Send + Sync + 'static,
{
    if id.is_empty() {
        return Err("GameTest function id cannot be empty".to_string());
    }

    let handler_id = {
        let mut handlers = TEST_FUNCTION_HANDLERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let handler_id = handlers.next_id;
        handlers.next_id = handlers.next_id.wrapping_add(1);
        handlers.handlers.insert(handler_id, Arc::new(handler));
        handler_id
    };

    if let Err(error) = wit::pumpkin::plugin::gametest::register_test_function(id, handler_id) {
        TEST_FUNCTION_HANDLERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .handlers
            .remove(&handler_id);
        return Err(error);
    }

    Ok(())
}

impl wit::exports::pumpkin::plugin::gametest_handler::Guest for Component {
    fn handle_test_function(handler_id: u32, test: Test) {
        let handler = TEST_FUNCTION_HANDLERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .handlers
            .get(&handler_id)
            .cloned();

        if let Some(handler) = handler {
            handler(test);
        }
    }
}
