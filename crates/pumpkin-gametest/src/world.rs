use async_trait::async_trait;
use pumpkin_data::BlockStateId;
use pumpkin_nbt::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::world::BlockFlags;

use crate::error::GameTestResult;

#[async_trait]
pub trait GameTestWorld: Send + Sync {
    async fn block_state_id(&self, position: &BlockPos) -> BlockStateId;

    async fn set_block_state(
        &self,
        position: &BlockPos,
        block_state_id: BlockStateId,
        flags: BlockFlags,
    ) -> GameTestResult<()>;

    async fn set_block_entity_nbt(
        &self,
        position: &BlockPos,
        nbt: &NbtCompound,
    ) -> GameTestResult<()>;

    async fn update_test_block_redstone(&self, position: &BlockPos) -> GameTestResult<()>;

    async fn trigger_test_block(&self, position: &BlockPos) -> GameTestResult<()>;

    async fn reset_test_block(&self, position: &BlockPos) -> GameTestResult<()>;

    async fn test_block_triggered(&self, position: &BlockPos) -> GameTestResult<bool>;

    async fn test_block_message(&self, position: &BlockPos) -> GameTestResult<String>;

    async fn surface_height(&self, x: i32, z: i32) -> i32;
}
