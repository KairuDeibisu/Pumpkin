use serde::{Deserialize, Serialize};

/// This configuration controls the behavior of the Pumpkin `GameTest` framework.
#[derive(Deserialize, Serialize)]
#[serde(default)]
#[derive(Default)]
pub struct GameTestConfig {
    /// Whether to load example tests from the `pumpkin-unit-test-example` datapack.    
    pub load_example_tests: bool,
}

