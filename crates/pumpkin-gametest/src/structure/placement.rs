use pumpkin_data::Block;
use pumpkin_nbt::{NbtCompound, tag::NbtTag};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use pumpkin_world::world::BlockFlags;

use crate::error::GameTestResult;
use crate::model::TestRotation;
use crate::structure::template::StructureTemplate;
use crate::world::GameTestWorld;

const STRUCTURE_OFFSET: [i32; 3] = [0, 1, 1];

#[derive(Clone, Debug)]
pub struct PlacedStructure {
    test_instance_pos: BlockPos,
    origin: BlockPos,
    source_size: [i32; 3],
    size: [i32; 3],
    rotation: TestRotation,
}

impl PlacedStructure {
    #[must_use]
    pub const fn new(
        test_instance_pos: BlockPos,
        origin: BlockPos,
        source_size: [i32; 3],
        size: [i32; 3],
        rotation: TestRotation,
    ) -> Self {
        Self {
            test_instance_pos,
            origin,
            source_size,
            size,
            rotation,
        }
    }

    #[must_use]
    pub const fn test_instance_pos(&self) -> &BlockPos {
        &self.test_instance_pos
    }

    #[must_use]
    pub const fn origin(&self) -> &BlockPos {
        &self.origin
    }

    #[must_use]
    pub const fn size(&self) -> [i32; 3] {
        self.size
    }

    #[must_use]
    pub const fn rotation(&self) -> TestRotation {
        self.rotation
    }

    #[must_use]
    pub fn transform(&self, relative: &BlockPos) -> BlockPos {
        let transformed = self.rotation.as_block_rotation().transform_pos(
            Vector3::new(relative.0.x, relative.0.y, relative.0.z),
            Vector3::new(self.source_size[0], self.source_size[1], self.source_size[2]),
        );
        BlockPos::new(
            self.origin.0.x + transformed.x,
            self.origin.0.y + transformed.y,
            self.origin.0.z + transformed.z,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlacementPhase {
    Clearing,
    Controller,
    Blocks,
    Complete,
}

/// Incremental structure placement used by the server ticker.
///
/// Pumpkin's block mutation path performs substantially more work than vanilla's
/// direct chunk placement. Performing a full clear + structure placement in one
/// GameTest tick can therefore monopolize the authoritative server tick. This job
/// deliberately advances in bounded slices while remaining on that same tick loop.
pub struct StructurePlacement {
    test_id: String,
    placed: PlacedStructure,
    clear_origin: BlockPos,
    clear_size: [i32; 3],
    clear_index: usize,
    block_index: usize,
    phase: PlacementPhase,
}

impl StructurePlacement {
    pub async fn new(
        world: &dyn GameTestWorld,
        template: &StructureTemplate,
        test_id: &str,
        rotation: TestRotation,
        test_x: i32,
        test_y: Option<i32>,
        test_z: i32,
        padding: i32,
    ) -> GameTestResult<Self> {
        let test_y = match test_y {
            Some(test_y) => test_y,
            None => world.surface_height(test_x, test_z).await + 1,
        };
        let test_instance_pos = BlockPos::new(test_x, test_y, test_z);
        let origin = BlockPos::new(
            test_instance_pos.0.x + padding + STRUCTURE_OFFSET[0],
            test_instance_pos.0.y + padding + STRUCTURE_OFFSET[1],
            test_instance_pos.0.z + padding + STRUCTURE_OFFSET[2],
        );

        let source_size = template.size();
        let rotated_size = rotation
            .as_block_rotation()
            .transform_size(Vector3::new(source_size[0], source_size[1], source_size[2]));
        let size = [rotated_size.x, rotated_size.y, rotated_size.z];
        let diameter = padding.saturating_mul(2);
        let clear_origin = BlockPos::new(
            origin.0.x - padding,
            origin.0.y - padding,
            origin.0.z - padding,
        );
        let clear_size = [
            size[0].saturating_add(diameter),
            size[1].saturating_add(diameter),
            size[2].saturating_add(diameter),
        ];

        Ok(Self {
            test_id: test_id.to_string(),
            placed: PlacedStructure::new(
                test_instance_pos,
                origin,
                source_size,
                size,
                rotation,
            ),
            clear_origin,
            clear_size,
            clear_index: 0,
            block_index: 0,
            phase: PlacementPhase::Clearing,
        })
    }

    #[must_use]
    pub const fn test_instance_pos(&self) -> &BlockPos {
        self.placed.test_instance_pos()
    }

    /// Advances at most `work_budget` clear cells / controller / structure blocks.
    /// Returns the completed placement once all work has been committed.
    pub async fn advance(
        &mut self,
        world: &dyn GameTestWorld,
        template: &StructureTemplate,
        work_budget: usize,
    ) -> GameTestResult<Option<PlacedStructure>> {
        if work_budget == 0 {
            return Ok(None);
        }

        let mut work = 0usize;
        loop {
            match self.phase {
                PlacementPhase::Clearing => {
                    let total = box_volume(self.clear_size);
                    while self.clear_index < total && work < work_budget {
                        let position = position_at(
                            &self.clear_origin,
                            self.clear_size,
                            self.clear_index,
                        );
                        self.clear_index += 1;
                        work += 1;

                        // Avoid an expensive block mutation + client update when the
                        // target cell is already air. The read itself is still counted
                        // against the per-tick budget so a large empty volume cannot
                        // monopolize a tick either.
                        if world.block_state_id(&position).await != Block::AIR.default_state.id {
                            world
                                .set_block_state(
                                    &position,
                                    Block::AIR.default_state.id,
                                    clear_flags(),
                                )
                                .await?;
                        }
                    }

                    if self.clear_index < total {
                        return Ok(None);
                    }
                    self.phase = PlacementPhase::Controller;
                }
                PlacementPhase::Controller => {
                    if work >= work_budget {
                        return Ok(None);
                    }
                    work += 1;

                    world
                        .set_block_state(
                            self.placed.test_instance_pos(),
                            Block::TEST_INSTANCE_BLOCK.default_state.id,
                            BlockFlags::NOTIFY_ALL,
                        )
                        .await?;

                    let mut data = NbtCompound::new();
                    data.put_string("test", self.test_id.clone());
                    data.put(
                        "size",
                        NbtTag::IntArray(self.placed.source_size.to_vec()),
                    );
                    // The stored controller rotation is the extra rotation. The test
                    // definition's base rotation is applied to structure placement.
                    data.put_string(
                        "rotation",
                        TestRotation::None.serialized_name().to_string(),
                    );
                    data.put_bool("ignore_entities", false);
                    data.put_string("status", "cleared".to_string());

                    let mut test_instance_nbt = NbtCompound::new();
                    test_instance_nbt
                        .put_string("id", "minecraft:test_instance_block".to_string());
                    test_instance_nbt.put_compound("data", data);
                    world
                        .set_block_entity_nbt(
                            self.placed.test_instance_pos(),
                            &test_instance_nbt,
                        )
                        .await?;
                    self.phase = PlacementPhase::Blocks;
                }
                PlacementPhase::Blocks => {
                    while self.block_index < template.blocks().len() && work < work_budget {
                        let block = &template.blocks()[self.block_index];
                        self.block_index += 1;
                        work += 1;

                        let position = self.placed.transform(&BlockPos::new(
                            block.position[0],
                            block.position[1],
                            block.position[2],
                        ));
                        let state = world
                            .rotate_block_state(block.state, self.placed.rotation)
                            .await?;
                        world
                            .set_block_state(&position, state, place_flags())
                            .await?;

                        if let Some(nbt) = &block.nbt {
                            world.set_block_entity_nbt(&position, nbt).await?;
                        }
                    }

                    if self.block_index < template.blocks().len() {
                        return Ok(None);
                    }
                    self.phase = PlacementPhase::Complete;
                }
                PlacementPhase::Complete => return Ok(Some(self.placed.clone())),
            }
        }
    }
}

/// Compatibility helper for callers that explicitly need immediate placement.
/// GameTest ticker execution should use [`StructurePlacement`] so placement is
/// bounded across server ticks.
pub async fn place_structure(
    world: &dyn GameTestWorld,
    template: &StructureTemplate,
    test_id: &str,
    rotation: TestRotation,
    test_x: i32,
    test_y: Option<i32>,
    test_z: i32,
    padding: i32,
) -> GameTestResult<PlacedStructure> {
    let mut placement = StructurePlacement::new(
        world, template, test_id, rotation, test_x, test_y, test_z, padding,
    )
    .await?;

    loop {
        if let Some(placed) = placement.advance(world, template, usize::MAX).await? {
            return Ok(placed);
        }
    }
}

pub async fn clear_structure_area(
    world: &dyn GameTestWorld,
    origin: &BlockPos,
    size: [i32; 3],
) -> GameTestResult<()> {
    clear_box(world, origin, size).await
}

async fn clear_box(
    world: &dyn GameTestWorld,
    origin: &BlockPos,
    size: [i32; 3],
) -> GameTestResult<()> {
    let total = box_volume(size);
    for index in 0..total {
        let position = position_at(origin, size, index);
        if world.block_state_id(&position).await != Block::AIR.default_state.id {
            world
                .set_block_state(&position, Block::AIR.default_state.id, clear_flags())
                .await?;
        }
    }
    Ok(())
}

fn box_volume(size: [i32; 3]) -> usize {
    let x = usize::try_from(size[0].max(0)).unwrap_or(0);
    let y = usize::try_from(size[1].max(0)).unwrap_or(0);
    let z = usize::try_from(size[2].max(0)).unwrap_or(0);
    x.saturating_mul(y).saturating_mul(z)
}

fn position_at(origin: &BlockPos, size: [i32; 3], index: usize) -> BlockPos {
    let x_size = usize::try_from(size[0].max(1)).unwrap_or(1);
    let z_size = usize::try_from(size[2].max(1)).unwrap_or(1);
    let layer = x_size.saturating_mul(z_size).max(1);
    let y = index / layer;
    let in_layer = index % layer;
    let z = in_layer / x_size;
    let x = in_layer % x_size;

    BlockPos::new(
        origin.0.x + i32::try_from(x).unwrap_or(i32::MAX),
        origin.0.y + i32::try_from(y).unwrap_or(i32::MAX),
        origin.0.z + i32::try_from(z).unwrap_or(i32::MAX),
    )
}

fn place_flags() -> BlockFlags {
    BlockFlags::NOTIFY_LISTENERS
        | BlockFlags::MOVED
        | BlockFlags::SKIP_REDSTONE_WIRE_STATE_REPLACEMENT
        | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK
}

fn clear_flags() -> BlockFlags {
    BlockFlags::NOTIFY_LISTENERS
        | BlockFlags::SKIP_DROPS
        | BlockFlags::SKIP_REDSTONE_WIRE_STATE_REPLACEMENT
        | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK
}
