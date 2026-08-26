use pumpkin_data::Block;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use crate::error::GameTestResult;
use crate::structure::template::StructureTemplate;
use crate::world::GameTestWorld;

const STRUCTURE_OFFSET: [i32; 3] = [0, 1, 1];

#[derive(Clone, Debug)]
pub struct PlacedStructure {
    origin: BlockPos,
    size: [i32; 3],
}

impl PlacedStructure {
    #[must_use]
    pub const fn new(origin: BlockPos, size: [i32; 3]) -> Self {
        Self { origin, size }
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
    pub fn transform(&self, relative: &BlockPos) -> BlockPos {
        BlockPos::new(
            self.origin.0.x + relative.0.x,
            self.origin.0.y + relative.0.y,
            self.origin.0.z + relative.0.z,
        )
    }
}

pub async fn place_structure(
    world: &dyn GameTestWorld,
    template: &StructureTemplate,
    test_x: i32,
    test_z: i32,
    padding: i32,
) -> GameTestResult<PlacedStructure> {
    // Pumpkin's heightmap accessor returns the top occupied Y; vanilla's
    // getHeightmapPos uses the first free Y above it.
    let test_y = world.surface_height(test_x, test_z).await + 1;
    let origin = BlockPos::new(
        test_x + padding + STRUCTURE_OFFSET[0],
        test_y + padding + STRUCTURE_OFFSET[1],
        test_z + padding + STRUCTURE_OFFSET[2],
    );

    clear_structure_area(world, &origin, template.size()).await?;

    // StructureTemplate::placeInWorld uses listener updates while suppressing
    // immediate shape/redstone callbacks. These are Pumpkin's closest matching
    // flags and keep command/test/redstone blocks inert while the template appears.
    let place_flags = BlockFlags::NOTIFY_LISTENERS
        | BlockFlags::MOVED
        | BlockFlags::SKIP_REDSTONE_WIRE_STATE_REPLACEMENT
        | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK;

    for block in template.blocks() {
        let position = BlockPos::new(
            origin.0.x + block.position[0],
            origin.0.y + block.position[1],
            origin.0.z + block.position[2],
        );
        world
            .set_block_state(&position, block.state, place_flags)
            .await?;
    }

    Ok(PlacedStructure::new(origin, template.size()))
}

pub async fn clear_structure_area(
    world: &dyn GameTestWorld,
    origin: &BlockPos,
    size: [i32; 3],
) -> GameTestResult<()> {
    let clear_flags = BlockFlags::NOTIFY_LISTENERS
        | BlockFlags::SKIP_DROPS
        | BlockFlags::SKIP_REDSTONE_WIRE_STATE_REPLACEMENT
        | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK;

    for x in 0..size[0] {
        for y in 0..size[1] {
            for z in 0..size[2] {
                let position = BlockPos::new(origin.0.x + x, origin.0.y + y, origin.0.z + z);
                world
                    .set_block_state(&position, Block::AIR.default_state.id, clear_flags)
                    .await?;
            }
        }
    }

    Ok(())
}
