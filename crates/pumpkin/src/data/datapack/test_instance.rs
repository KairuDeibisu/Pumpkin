use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

pub type TestInstanceRegistry = HashMap<String, TestInstance>;

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
pub struct TestInstance {
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

impl TestInstance {
    fn validate(&self) -> bool {
        self.max_ticks > 0
            && self.setup_ticks >= 0
            && self.max_attempts > 0
            && self.required_successes > 0
            && (0..=128).contains(&self.padding)
    }
}

pub fn load_test_instances_from_dir(
    namespace: &str,
    test_instance_dir: &Path,
    registry: &mut TestInstanceRegistry,
) -> usize {
    if !test_instance_dir.is_dir() {
        return 0;
    }

    let before = registry.len();
    load_test_instances_recursive(namespace, test_instance_dir, test_instance_dir, registry);
    registry.len() - before
}

fn load_test_instances_recursive(
    namespace: &str,
    base_dir: &Path,
    current_dir: &Path,
    registry: &mut TestInstanceRegistry,
) {
    let Ok(entries) = fs::read_dir(current_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            load_test_instances_recursive(namespace, base_dir, &path, registry);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            && let Ok(rel_path) = path.strip_prefix(base_dir)
        {
            let mut stem_path = rel_path.to_string_lossy().to_string();

            if let Some(stem) = stem_path.strip_suffix(".json") {
                stem_path = stem.to_string();
            }

            let stem_path = stem_path.replace('\\', "/");
            let test_instance_id = format!("{namespace}:{stem_path}");

            if let Ok(content) = fs::read_to_string(&path)
                && let Ok(instance) = serde_json::from_str::<TestInstance>(&content)
                && instance.validate()
            {
                registry.insert(test_instance_id, instance);
            }
        }
    }
}

const fn default_true() -> bool {
    true
}

const fn default_one() -> i32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_block_based_test_instance_with_vanilla_defaults() {
        let dir = tempfile::tempdir().expect("temp dir");
        let nested = dir.path().join("mob_ai");

        fs::create_dir_all(&nested).expect("create nested test_instance dir");

        fs::write(
            nested.join("some_ai_test.json"),
            r#"{
                "type": "minecraft:block_based",
                "environment": "minecraft:default",
                "structure": "pumpkin:creeper_should_run_from_cat",
                "max_ticks": 200
            }"#,
        )
        .expect("write test instance");

        let mut registry = TestInstanceRegistry::new();
        let loaded =
            load_test_instances_from_dir("pumpkin", dir.path(), &mut registry);

        assert_eq!(loaded, 1);

        let instance = registry
            .get("pumpkin:mob_ai/some_ai_test")
            .expect("test instance should be registered");

        assert_eq!(instance.instance_type, TestType::BlockBased);
        assert_eq!(
            instance.environment,
            Value::String("minecraft:default".into())
        );
        assert_eq!(
            instance.structure,
            "pumpkin:some_ai_test"
        );
        assert_eq!(instance.max_ticks, 200);
        assert_eq!(instance.setup_ticks, 0);
        assert!(instance.required);
        assert_eq!(instance.rotation, TestRotation::None);
        assert!(!instance.manual_only);
        assert_eq!(instance.max_attempts, 1);
        assert_eq!(instance.required_successes, 1);
        assert!(!instance.sky_access);
        assert_eq!(instance.padding, 0);
    }
}