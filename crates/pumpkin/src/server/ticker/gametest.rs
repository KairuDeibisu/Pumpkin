use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use pumpkin_data::{BlockState, BlockStateId};
use pumpkin_gametest::{
    BlockBasedTest, GameTestError, GameTestResult, GameTestWorld, StructureTemplate, TestRotation,
    TestRun, TestRunner,
};
use pumpkin_nbt::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_world::{chunk::ChunkHeightmapType, world::BlockFlags};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::{
    block::entities::{
        BlockEntity, block_entity_from_nbt, test_block::TestBlockBlockEntity,
        test_instance_block::TestInstanceBlockBlockEntity,
    },
    server::Server,
    world::World,
};

static GAME_TEST_QUEUE: LazyLock<Mutex<Vec<GameTestRequest>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// A request to start a GameTest.
///
/// This is deliberately command-agnostic and runtime-light: producers choose a
/// test id, world, and anchor coordinates. Definition/structure loading, world
/// adaptation, controller state, START pulses, retries, and completion all belong
/// to the GameTest runtime owned by the server ticker.
pub struct GameTestRequest {
    test_id: String,
    world: Arc<World>,
    test_x: i32,
    test_z: i32,
}

impl GameTestRequest {
    #[must_use]
    pub fn new(test_id: impl Into<String>, world: Arc<World>, test_x: i32, test_z: i32) -> Self {
        Self {
            test_id: test_id.into(),
            world,
            test_x,
            test_z,
        }
    }
}

pub async fn enqueue_game_test(request: GameTestRequest) {
    GAME_TEST_QUEUE.lock().await.push(request);
}

pub(super) async fn drain_game_test_queue(server: &Arc<Server>, runner: &mut TestRunner) {
    let queued = {
        let mut queue = GAME_TEST_QUEUE.lock().await;
        std::mem::take(&mut *queue)
    };

    for request in queued {
        let test_id = request.test_id.clone();
        match prepare_test_run(server, request).await {
            Ok(run) => {
                info!(target: "pumpkin::gametest", test = %test_id, "Starting queued GameTest");
                runner.enqueue(run);
            }
            Err(error) => {
                warn!(
                    target: "pumpkin::gametest",
                    test = %test_id,
                    error = %error,
                    "Unable to start queued GameTest"
                );
            }
        }
    }
}

async fn prepare_test_run(server: &Arc<Server>, request: GameTestRequest) -> GameTestResult<TestRun> {
    let test_instance = server
        .datapack_manager
        .get_test_instance(&request.test_id)
        .await
        .ok_or_else(|| GameTestError::World(format!("Unknown test instance '{}'", request.test_id)))?;

    let structure = server
        .datapack_manager
        .load_structure(&test_instance.structure)
        .await
        .map_err(GameTestError::World)?;
    let template = StructureTemplate::from_nbt(&structure)?;
    let test = BlockBasedTest::new(request.test_id, test_instance);
    let world: Arc<dyn GameTestWorld> = Arc::new(ServerGameTestWorld {
        world: request.world,
    });

    Ok(TestRun::new(
        test,
        world,
        Arc::new(template),
        request.test_x,
        request.test_z,
    ))
}

struct ServerGameTestWorld {
    world: Arc<World>,
}

impl ServerGameTestWorld {
    fn test_block_entity(&self, position: &BlockPos) -> GameTestResult<Arc<TestBlockBlockEntity>> {
        let entity = self.world.get_block_entity(position).ok_or_else(|| {
            GameTestError::World(format!("Missing test block entity at {position}"))
        })?;

        Arc::downcast::<TestBlockBlockEntity>(entity).map_err(|_| {
            GameTestError::World(format!("Block entity at {position} is not a test block"))
        })
    }

    fn test_instance_block_entity(
        &self,
        position: &BlockPos,
    ) -> GameTestResult<Arc<TestInstanceBlockBlockEntity>> {
        let entity = self.world.get_block_entity(position).ok_or_else(|| {
            GameTestError::World(format!("Missing test instance block entity at {position}"))
        })?;

        Arc::downcast::<TestInstanceBlockBlockEntity>(entity).map_err(|_| {
            GameTestError::World(format!(
                "Block entity at {position} is not a test instance block"
            ))
        })
    }

    fn sync_block_entity<T: BlockEntity + 'static>(&self, entity: Arc<T>) {
        let entity: Arc<dyn BlockEntity> = entity;
        self.world.update_block_entity(&entity);
    }
}

#[async_trait]
impl GameTestWorld for ServerGameTestWorld {
    async fn block_state_id(&self, position: &BlockPos) -> BlockStateId {
        self.world.get_block_state_id_async(position).await
    }

    async fn set_block_state(
        &self,
        position: &BlockPos,
        block_state_id: BlockStateId,
        flags: BlockFlags,
    ) -> GameTestResult<()> {
        self.world
            .set_block_state(position, block_state_id, flags)
            .await;
        Ok(())
    }

    async fn rotate_block_state(
        &self,
        block_state_id: BlockStateId,
        rotation: TestRotation,
    ) -> GameTestResult<BlockStateId> {
        let (block, _) = BlockState::from_id_with_block(block_state_id);
        Ok(self
            .world
            .block_registry
            .rotate(block, block_state_id, rotation.as_block_rotation())
            .id)
    }

    async fn set_block_entity_nbt(
        &self,
        position: &BlockPos,
        nbt: &NbtCompound,
    ) -> GameTestResult<()> {
        let mut nbt = nbt.clone();
        nbt.put_int("x", position.0.x);
        nbt.put_int("y", position.0.y);
        nbt.put_int("z", position.0.z);

        let entity = block_entity_from_nbt(&nbt).ok_or_else(|| {
            let id = nbt.get_string("id").unwrap_or("<missing id>");
            GameTestError::World(format!(
                "Unable to create block entity '{id}' at {position}"
            ))
        })?;

        self.world.remove_block_entity(position);
        // add_block_entity is the single installation/synchronization path. Calling
        // update_block_entity immediately afterwards sent an identical BE packet a
        // second time for every structure block entity.
        self.world.add_block_entity(entity);
        Ok(())
    }

    async fn set_test_instance_running(&self, position: &BlockPos) -> GameTestResult<()> {
        let entity = self.test_instance_block_entity(position)?;
        entity.clear_error_markers().await;
        entity.set_running().await;
        self.sync_block_entity(entity);
        Ok(())
    }

    async fn set_test_instance_success(&self, position: &BlockPos) -> GameTestResult<()> {
        let entity = self.test_instance_block_entity(position)?;
        entity.clear_error_markers().await;
        entity.set_success().await;
        self.sync_block_entity(entity);
        Ok(())
    }

    async fn set_test_instance_failure(
        &self,
        position: &BlockPos,
        message: &str,
        marker: Option<(BlockPos, String)>,
    ) -> GameTestResult<()> {
        let entity = self.test_instance_block_entity(position)?;
        entity.clear_error_markers().await;
        if let Some((marker_position, marker_text)) = marker {
            entity.mark_error(marker_position, marker_text).await;
        }
        entity.set_error_message(message.to_string()).await;
        self.sync_block_entity(entity);
        Ok(())
    }

    async fn trigger_test_block(&self, position: &BlockPos) -> GameTestResult<()> {
        self.test_block_entity(position)?
            .trigger(&self.world)
            .await;
        Ok(())
    }

    async fn reset_test_block(&self, position: &BlockPos) -> GameTestResult<()> {
        self.test_block_entity(position)?.reset(&self.world).await;
        Ok(())
    }

    async fn test_block_triggered(&self, position: &BlockPos) -> GameTestResult<bool> {
        Ok(self.test_block_entity(position)?.has_triggered())
    }

    async fn test_block_message(&self, position: &BlockPos) -> GameTestResult<String> {
        Ok(self.test_block_entity(position)?.message().await)
    }

    async fn surface_height(&self, x: i32, z: i32) -> i32 {
        self.world
            .get_heightmap_height_async(ChunkHeightmapType::WorldSurface, x, z)
            .await
    }
}
