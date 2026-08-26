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

pub async fn place_structure(
    world: &dyn GameTestWorld,
    template: &StructureTemplate,
    test_id: &str,
    rotation: TestRotation,
    test_x: i32,
    test_z: i32,
    padding: i32,
) -> GameTestResult<PlacedStructure> {
    // TestInstanceBlockEntity.getStructurePos offsets the controller by padding and
    // then by STRUCTURE_OFFSET. The controller itself is outside the structure box.
    let test_y = world.surface_height(test_x, test_z).await + 1;
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

    clear_test_area(world, &origin, size, padding).await?;

    world
        .set_block_state(
            &test_instance_pos,
            Block::TEST_INSTANCE_BLOCK.default_state.id,
            BlockFlags::NOTIFY_ALL,
        )
        .await?;

    // This mirrors TestInstanceBlockEntity.Data. The stored rotation is the extra
    // controller rotation. /test run has no extra rotation, so the test definition's
    // rotation is applied to the template while this remains "none" as in vanilla.
    let mut data = NbtCompound::new();
    data.put_string("test", test_id.to_string());
    data.put("size", NbtTag::IntArray(source_size.to_vec()));
    data.put_string("rotation", TestRotation::None.serialized_name().to_string());
    data.put_bool("ignore_entities", false);
    data.put_string("status", "cleared".to_string());

    let mut test_instance_nbt = NbtCompound::new();
    test_instance_nbt.put_string("id", "minecraft:test_instance_block".to_string());
    test_instance_nbt.put_compound("data", data);
    world
        .set_block_entity_nbt(&test_instance_pos, &test_instance_nbt)
        .await?;

    // StructureTemplate.placeInWorld rotates both relative positions and block
    // states before loading block-entity NBT at the transformed absolute position.
    let place_flags = BlockFlags::NOTIFY_LISTENERS
        | BlockFlags::MOVED
        | BlockFlags::SKIP_REDSTONE_WIRE_STATE_REPLACEMENT
        | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK;

    let source_size_vec = Vector3::new(source_size[0], source_size[1], source_size[2]);
    for block in template.blocks() {
        let transformed = rotation.as_block_rotation().transform_pos(
            Vector3::new(block.position[0], block.position[1], block.position[2]),
            source_size_vec,
        );
        let position = BlockPos::new(
            origin.0.x + transformed.x,
            origin.0.y + transformed.y,
            origin.0.z + transformed.z,
        );
        let state = world.rotate_block_state(block.state, rotation).await?;
        world.set_block_state(&position, state, place_flags).await?;

        if let Some(nbt) = &block.nbt {
            world.set_block_entity_nbt(&position, nbt).await?;
        }
    }

    Ok(PlacedStructure::new(
        test_instance_pos,
        origin,
        source_size,
        size,
        rotation,
    ))
}

pub async fn clear_structure_area(
    world: &dyn GameTestWorld,
    origin: &BlockPos,
    size: [i32; 3],
) -> GameTestResult<()> {
    clear_box(world, origin, size).await
}

async fn clear_test_area(
    world: &dyn GameTestWorld,
    origin: &BlockPos,
    size: [i32; 3],
    padding: i32,
) -> GameTestResult<()> {
    let min = BlockPos::new(
        origin.0.x - padding,
        origin.0.y - padding,
        origin.0.z - padding,
    );
    let diameter = padding.saturating_mul(2);
    clear_box(
        world,
        &min,
        [size[0] + diameter, size[1] + diameter, size[2] + diameter],
    )
    .await
}

async fn clear_box(
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
