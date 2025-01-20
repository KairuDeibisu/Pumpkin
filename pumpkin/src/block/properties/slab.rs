use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use pumpkin_util::math::position::BlockPos;
use pumpkin_world::block::block_registry::Block;
use pumpkin_world::block::{block_registry::Property, BlockFace};

use crate::{
    block::block_properties_manager::{
        get_property_key, BlockBehavior, BlockProperty, SlabPosition,
    },
    world::World,
};

pub static SLAB_BEHAVIOR: OnceLock<Arc<SlabBehavior>> = OnceLock::new();

// Example of a behavior with shared static data
pub struct SlabBehavior {
    // Shared static data for all slabs
    state_mappings: HashMap<Vec<String>, u16>,
    property_mappings: HashMap<u16, Vec<String>>,
}

impl SlabBehavior {
    pub fn get_or_init(properties: &[Property]) -> Arc<Self> {
        SLAB_BEHAVIOR
            .get_or_init(|| Arc::new(Self::new(properties)))
            .clone()
    }

    pub fn get() -> Arc<Self> {
        SLAB_BEHAVIOR.get().expect("Slab Uninitialized").clone()
    }

    pub fn new(properties: &[Property]) -> Self {
        let total_combinations: usize = properties.iter().map(|p| p.values.len()).product();

        let mut forward_map = HashMap::with_capacity(total_combinations);
        let mut reverse_map = HashMap::with_capacity(total_combinations);

        for i in 0..total_combinations {
            let mut current = i;
            let mut combination = Vec::with_capacity(properties.len());

            for property in properties.iter().rev() {
                let property_size = property.values.len();
                combination.push(current % property_size);
                current /= property_size;
            }

            combination.reverse();

            let key: Vec<String> = combination
                .iter()
                .enumerate()
                .map(|(prop_idx, &state_idx)| {
                    format!(
                        "{}{}",
                        properties[prop_idx].name, properties[prop_idx].values[state_idx]
                    )
                })
                .collect();

            forward_map.insert(key.clone(), i as u16);
            reverse_map.insert(i as u16, key);
        }

        Self {
            state_mappings: forward_map,
            property_mappings: reverse_map,
        }
    }

    pub fn evalute_property_type(
        block: &Block,
        clicked_block: &Block,
        _world_pos: &BlockPos,
        face: &BlockFace,
    ) -> String {
        if block.id == clicked_block.id && *face == BlockFace::Top {
            return format!("{}{}", "type", "double");
        }
        format!("{}{}", "type", "bottom")
    }

    pub fn evalute_property_waterlogged(
        _block: &Block,
        clicked_block: &Block,
        _world_pos: &BlockPos,
        _face: &BlockFace,
    ) -> String {
        if clicked_block.name == "water" {
            return format!("{}{}", "waterlogged", "true");
        }
        format!("{}{}", "waterlogged", "false")
    }
}

#[async_trait::async_trait]
impl BlockBehavior for SlabBehavior {
    async fn map_state_id(
        &self,
        world: &World,
        block: &Block,
        face: &BlockFace,
        world_pos: &BlockPos,
    ) -> u16 {
        let clicked_block = world.get_block(world_pos).await.unwrap();
        let mut hmap_key: Vec<String> = Vec::with_capacity(block.properties.len());
        let slab_behaviour = Self::get();

        for property in &block.properties {
            let state = match get_property_key(property.name.as_str()).expect("Property not found")
            {
                BlockProperty::SlabType(SlabPosition::Top) => {
                    Self::evalute_property_type(block, clicked_block, world_pos, face)
                }
                BlockProperty::Waterlogged(false) => {
                    Self::evalute_property_waterlogged(block, clicked_block, world_pos, face)
                }
                _ => panic!("Property not found"),
            };
            hmap_key.push(state.to_string());
        }

        // Base state id plus offset
        block.states[0].id + slab_behaviour.state_mappings[&hmap_key]
    }

    async fn is_updateable(
        &self,
        world: &World,
        block: &Block,
        face: &BlockFace,
        world_pos: &BlockPos,
    ) -> bool {
        let clicked_block = world.get_block(world_pos).await.unwrap();
        if block.id != clicked_block.id || *face != BlockFace::Top {
            return false;
        }

        let clicked_block_state_id = world.get_block_state_id(world_pos).await.unwrap();

        let key = clicked_block_state_id - clicked_block.states[0].id;
        if let Some(properties) = Self::get().property_mappings.get(&key) {
            log::warn!("Properties: {:?}", properties);
            if properties.contains(&"typebottom".to_string()) {
                return true;
            }
        }
        false
    }
}