pub mod function_loader;
pub mod recipe_loader;
pub mod test_instance;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use pumpkin_data::registry::RegistryEntryData;
use pumpkin_nbt::{NbtCompound, nbt_compress::read_gzip_compound_tag};
use pumpkin_protocol::codec::recipe::DynamicRecipe;

use crate::command::context::command_source::CommandSource;
use crate::server::Server;
use crate::server::recipe::RecipeManager;

use self::test_instance::{
    TestInstance, TestInstanceRegistry, load_test_instances_from_dir, to_registry_entry,
};

#[derive(Clone, Debug)]
pub struct LoadedDatapack {
    pub id: String,
    pub name: String,
    pub description: String,
    pub pack_format: u32,
    pub root_path: PathBuf,
    pub recipe_count: usize,
    pub function_count: usize,
}

pub struct DatapackManager {
    loaded_packs: RwLock<Vec<LoadedDatapack>>,
    functions: RwLock<HashMap<String, Vec<String>>>,
    function_tags: RwLock<HashMap<String, Vec<String>>>,
    test_instances: RwLock<TestInstanceRegistry>,
}

impl Default for DatapackManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DatapackManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            loaded_packs: RwLock::new(Vec::new()),
            functions: RwLock::new(HashMap::new()),
            function_tags: RwLock::new(HashMap::new()),
            test_instances: RwLock::new(HashMap::new()),
        }
    }

    pub async fn load_all(
        &self,
        world_path: &Path,
        enabled_packs: &[String],
        recipe_manager: &RecipeManager,
    ) {
        let datapacks_dir = world_path.join("datapacks");
        let mut loaded_packs_vec = Vec::new();
        let mut all_recipes: Vec<DynamicRecipe> = Vec::new();
        let mut all_functions: HashMap<String, Vec<String>> = HashMap::new();
        let mut all_function_tags: HashMap<String, Vec<String>> = HashMap::new();
        let mut all_test_instances = TestInstanceRegistry::new();

        if datapacks_dir.is_dir() {
            let Ok(entries) = fs::read_dir(&datapacks_dir) else {
                warn!(
                    "Failed to read datapacks directory: {}",
                    datapacks_dir.display()
                );
                return;
            };

            for entry in entries.flatten() {
                let pack_path = entry.path();
                let file_name = entry.file_name().to_string_lossy().to_string();

                if file_name.starts_with('.') || !pack_path.is_dir() {
                    continue;
                }

                let pack_id = format!("file/{file_name}");
                let is_enabled = enabled_packs
                    .iter()
                    .any(|p| p == &pack_id || p == &file_name);
                if !is_enabled {
                    continue;
                }

                let (description, pack_format) = read_pack_mcmeta(&pack_path);

                let data_dir = pack_path.join("data");
                let mut pack_recipe_count = 0;
                let mut pack_function_count = 0;
                let mut pack_test_instance_count = 0;

                if data_dir.is_dir()
                    && let Ok(ns_entries) = fs::read_dir(&data_dir)
                {
                    for ns_entry in ns_entries.flatten() {
                        let ns_path = ns_entry.path();
                        if !ns_path.is_dir() {
                            continue;
                        }
                        let namespace = ns_entry.file_name().to_string_lossy().to_string();

                        // Load recipes
                        for recipe_sub in ["recipe", "recipes"] {
                            let recipe_dir = ns_path.join(recipe_sub);
                            if recipe_dir.is_dir() {
                                load_recipes_from_dir(
                                    &namespace,
                                    &recipe_dir,
                                    &mut all_recipes,
                                    &mut pack_recipe_count,
                                );
                            }
                        }

                        // Load functions
                        for fn_sub in ["function", "functions"] {
                            let fn_dir = ns_path.join(fn_sub);
                            if fn_dir.is_dir() {
                                let before = all_functions.len();
                                function_loader::load_functions_from_dir(
                                    &namespace,
                                    &fn_dir,
                                    &mut all_functions,
                                );
                                pack_function_count += all_functions.len() - before;
                            }
                        }

                        // Load tags
                        let tags_dir = ns_path.join("tags");
                        if tags_dir.is_dir() {
                            function_loader::load_function_tags_from_dir(
                                &namespace,
                                &tags_dir,
                                &mut all_function_tags,
                            );
                        }

                        // Load game test instances
                        let test_instance_dir = ns_path.join("test_instance");
                        if test_instance_dir.is_dir() {
                            pack_test_instance_count += load_test_instances_from_dir(
                                &namespace,
                                &test_instance_dir,
                                &mut all_test_instances,
                            );
                        }
                    }
                }

                info!(
                    "Loaded datapack '{file_name}': {pack_recipe_count} recipe(s), {pack_function_count} function(s), {pack_test_instance_count} test instance(s)"
                );

                loaded_packs_vec.push(LoadedDatapack {
                    id: pack_id,
                    name: file_name,
                    description,
                    pack_format,
                    root_path: pack_path,
                    recipe_count: pack_recipe_count,
                    function_count: pack_function_count,
                });
            }
        }

        recipe_manager.set_recipes(all_recipes).await;
        *self.loaded_packs.write().await = loaded_packs_vec;
        *self.functions.write().await = all_functions;
        *self.function_tags.write().await = all_function_tags;
        *self.test_instances.write().await = all_test_instances;
    }

    pub async fn get_loaded_packs(&self) -> Vec<LoadedDatapack> {
        self.loaded_packs.read().await.clone()
    }

    pub async fn get_functions(&self) -> HashMap<String, Vec<String>> {
        self.functions.read().await.clone()
    }

    pub async fn get_test_instance(&self, name: &str) -> Option<TestInstance> {
        self.test_instances.read().await.get(name).cloned()
    }

    pub async fn get_test_instance_names(&self) -> Vec<String> {
        let test_instances = self.test_instances.read().await;
        let mut names: Vec<_> = test_instances.keys().cloned().collect();
        names.sort_unstable();
        names
    }

    /// Returns datapack test instances in the protocol's synced-registry entry format.
    /// The vanilla Test Instance Block renderer resolves required/padding/base rotation
    /// through this registry using the controller's `data.test` resource key.
    pub async fn get_test_instance_registry_entries(&self) -> Vec<RegistryEntryData> {
        let test_instances = self.test_instances.read().await;
        let mut entries: Vec<_> = test_instances
            .iter()
            .map(|(id, instance)| to_registry_entry(id.clone(), instance))
            .collect();
        entries.sort_unstable_by(|left, right| left.entry_id.cmp(&right.entry_id));
        entries
    }

    /// Loads a Java Edition structure NBT from the currently enabled datapacks.
    ///
    /// Structure identifiers are resource locations such as
    /// `minecraft:village/plains/houses/plains_small_house_1`. Both the current
    /// `structure` directory and the legacy `structures` directory are checked.
    /// Bedrock `.mcstructure` files are intentionally not supported.
    pub async fn load_structure(&self, resource_location: &str) -> Result<NbtCompound, String> {
        let (namespace, path) = parse_structure_resource_location(resource_location)?;

        let loaded_packs = self.loaded_packs.read().await;
        let mut nbt_path = None;
        let mut unsupported_path = None;

        // Later-loaded packs take precedence, matching the overwrite behavior
        // used by the other datapack registries.
        'packs: for pack in loaded_packs.iter().rev() {
            for structure_dir in ["structure", "structures"] {
                let base = pack
                    .root_path
                    .join("data")
                    .join(namespace)
                    .join(structure_dir);

                let candidate = base.join(format!("{path}.nbt"));
                if candidate.is_file() {
                    nbt_path = Some(candidate);
                    break 'packs;
                }

                let candidate = base.join(format!("{path}.mcstructure"));
                if unsupported_path.is_none() && candidate.is_file() {
                    unsupported_path = Some(candidate);
                }
            }
        }
        drop(loaded_packs);

        let Some(nbt_path) = nbt_path else {
            if let Some(path) = unsupported_path {
                return Err(format!(
                    "Bedrock .mcstructure files are not supported: {}",
                    path.display()
                ));
            }
            return Err(format!(
                "Structure '{resource_location}' was not found in any enabled datapack"
            ));
        };

        // Gzip/NBT decoding is synchronous, so keep it off the async server task.
        let display_path = nbt_path.display().to_string();
        tokio::task::spawn_blocking(move || {
            let file = fs::File::open(&nbt_path)
                .map_err(|error| format!("Failed to open structure '{display_path}': {error}"))?;
            read_gzip_compound_tag(file)
                .map_err(|error| format!("Failed to parse structure '{display_path}': {error}"))
        })
        .await
        .map_err(|error| format!("Structure loader task failed: {error}"))?
    }

    pub async fn get_function_names(&self) -> Vec<String> {
        let fns = self.functions.read().await;
        let tags = self.function_tags.read().await;
        let mut names = Vec::with_capacity(fns.len() + tags.len());
        names.extend(fns.keys().cloned());
        for tag in tags.keys() {
            names.push(format!("#{tag}"));
        }
        names
    }

    pub async fn execute_function(
        &self,
        server: &Arc<Server>,
        source: &CommandSource,
        name: &str,
    ) -> Result<usize, String> {
        let (functions_to_run, is_tag) = if let Some(tag_name) = name.strip_prefix('#') {
            let tags = self.function_tags.read().await;
            let Some(fns) = tags.get(tag_name) else {
                return Err(format!("Unknown function tag: #{tag_name}"));
            };
            (fns.clone(), true)
        } else {
            (vec![name.to_string()], false)
        };

        let all_fns = self.functions.read().await;
        let mut total_executed = 0;

        for fn_id in functions_to_run {
            let Some(lines) = all_fns.get(&fn_id) else {
                if !is_tag {
                    return Err(format!("Unknown function: {fn_id}"));
                }
                continue;
            };

            for line in lines {
                server
                    .command_dispatcher
                    .load()
                    .handle_command(source, line)
                    .await;
                total_executed += 1;
            }
        }

        Ok(total_executed)
    }
}

fn parse_structure_resource_location(resource_location: &str) -> Result<(&str, &str), String> {
    let (namespace, raw_path) = resource_location
        .split_once(':')
        .unwrap_or(("minecraft", resource_location));

    if namespace.is_empty()
        || !namespace.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.')
        })
    {
        return Err(format!("Invalid structure namespace in '{resource_location}'"));
    }

    if raw_path.ends_with(".mcstructure") {
        return Err("Bedrock .mcstructure files are not supported".to_string());
    }

    let path = raw_path.strip_suffix(".nbt").unwrap_or(raw_path);
    if path.is_empty()
        || path.split('/').any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || !path.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.' | b'/')
        })
    {
        return Err(format!("Invalid structure path in '{resource_location}'"));
    }

    Ok((namespace, path))
}

fn read_pack_mcmeta(pack_path: &Path) -> (String, u32) {
    let mcmeta_path = pack_path.join("pack.mcmeta");
    if let Ok(content) = fs::read_to_string(mcmeta_path)
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
    {
        let pack = val.get("pack");
        let description = pack
            .and_then(|p| p.get("description"))
            .map(|d| {
                d.as_str()
                    .map_or_else(|| d.to_string(), ToString::to_string)
            })
            .unwrap_or_default();
        let pack_format = pack
            .and_then(|p| p.get("pack_format"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(61) as u32;
        return (description, pack_format);
    }
    (String::new(), 61)
}

fn load_recipes_from_dir(
    namespace: &str,
    dir: &Path,
    all_recipes: &mut Vec<DynamicRecipe>,
    count: &mut usize,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            load_recipes_from_dir(namespace, &path, all_recipes, count);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if let Ok(content) = fs::read_to_string(&path)
                && let Some(recipe) = recipe_loader::parse_recipe(namespace, &stem, &content)
            {
                all_recipes.push(recipe);
                *count += 1;
            }
        }
    }
}
