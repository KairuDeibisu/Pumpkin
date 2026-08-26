use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum TestType {
    #[serde(rename = "minecraft:block_based")]
    BlockBased,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
pub enum TestRotation {
    #[default]
    #[serde(rename = "none")]
    None,
    #[serde(rename = "clockwise_90")]
    Clockwise90,
    #[serde(rename = "180")]
    Clockwise180,
    #[serde(rename = "counterclockwise_90")]
    Counterclockwise90,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct TestDefinition {
    #[serde(rename = "type")]
    pub instance_type: TestType,
    pub environment: Value,
    pub structure: String,
    pub max_ticks: i32,
    #[serde(default)]
    pub setup_ticks: i32,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub rotation: TestRotation,
    #[serde(default)]
    pub manual_only: bool,
    #[serde(default = "default_one")]
    pub max_attempts: i32,
    #[serde(default = "default_one")]
    pub required_successes: i32,
    #[serde(default)]
    pub sky_access: bool,
    #[serde(default)]
    pub padding: i32,
}

impl TestDefinition {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.max_ticks > 0
            && self.setup_ticks >= 0
            && self.max_attempts > 0
            && self.required_successes > 0
            && (0..=128).contains(&self.padding)
    }
}

const fn default_true() -> bool {
    true
}

const fn default_one() -> i32 {
    1
}
