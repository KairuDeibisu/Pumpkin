use serde::{Deserialize, Serialize};

/// This configuration controls the behavior of the Pumpkin GameTest framework. 
#[derive(Deserialize, Serialize)]
#[serde(default)]
pub struct GameTestConfig {
    /// Whether to load example tests from the `pumpkin-unit-test-example` datapack.    
    pub load_example_tests: bool,
}

impl Default for GameTestConfig {
    fn default() -> Self {
        Self { load_example_tests: false }
    }
}
