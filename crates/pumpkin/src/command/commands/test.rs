use std::sync::Arc;

use async_trait::async_trait;
use pumpkin_data::BlockStateId;
use pumpkin_gametest::{GameTestResult, GameTestWorld, StructureTemplate, place_structure};
use pumpkin_util::PermissionLvl;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
use pumpkin_world::chunk::ChunkHeightmapType;
use pumpkin_world::world::BlockFlags;
use tracing::info;

use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::core::string::StringArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::command::suggestion::provider::{SuggestionProvider, SuggestionProviderResult};
use crate::command::suggestion::suggestions::SuggestionsBuilder;
use crate::world::World;

const DESCRIPTION: &str = "Runs a GameTest test instance.";
const PERMISSION: &str = "minecraft:command.test";
const ARG_NAME: &str = "name";
const TEST_POS_Z_OFFSET_FROM_PLAYER: i32 = 3;

struct TestInstanceSuggestionProvider;

impl SuggestionProvider for TestInstanceSuggestionProvider {
    fn suggest<'a>(
        &'a self,
        context: &'a CommandContext,
        mut builder: SuggestionsBuilder,
    ) -> SuggestionProviderResult<'a> {
        Box::pin(async move {
            for name in context.server().datapack_manager.get_test_instance_names().await {
                builder = builder.suggest(name);
            }
            builder.build()
        })
    }
}

struct CommandGameTestWorld<'a> {
    world: &'a Arc<World>,
}

#[async_trait]
impl GameTestWorld for CommandGameTestWorld<'_> {
    async fn block_state_id(&self, position: &BlockPos) -> BlockStateId {
        self.world.get_block_state_id_async(position).await
    }

    async fn set_block_state(
        &self,
        position: &BlockPos,
        block_state_id: BlockStateId,
        flags: BlockFlags,
    ) -> GameTestResult<()> {
        World::set_block_state(self.world, position, block_state_id, flags).await;
        Ok(())
    }

    async fn surface_height(&self, x: i32, z: i32) -> i32 {
        self.world
            .get_heightmap_height_async(ChunkHeightmapType::WorldSurface, x, z)
            .await
    }
}

struct RunExecutor;

impl CommandExecutor for RunExecutor {
    fn execute<'a>(&'a self, context: &'a CommandContext) -> CommandExecutorResult<'a> {
        Box::pin(async move {
            let name = StringArgumentType::get(context, ARG_NAME)?;
            let server = context.server();

            let Some(test_instance) = server.datapack_manager.get_test_instance(name).await else {
                context
                    .source
                    .send_error(TextComponent::text(format!(
                        "Unknown test instance '{name}'"
                    )))
                    .await;
                return Ok(0);
            };

            let structure = match server
                .datapack_manager
                .load_structure(&test_instance.structure)
                .await
            {
                Ok(structure) => structure,
                Err(error) => {
                    context
                        .source
                        .send_error(TextComponent::text(format!(
                            "Failed to load test instance '{name}': {error}"
                        )))
                        .await;
                    return Ok(0);
                }
            };

            let template = match StructureTemplate::from_nbt(&structure) {
                Ok(template) => template,
                Err(error) => {
                    context
                        .source
                        .send_error(TextComponent::text(format!(
                            "Failed to place test instance '{name}': {error}"
                        )))
                        .await;
                    return Ok(0);
                }
            };

            let source_pos = &context.source.position;
            let test_x = source_pos.x.floor() as i32;
            let test_z = source_pos.z.floor() as i32 + TEST_POS_Z_OFFSET_FROM_PLAYER;
            let game_test_world = CommandGameTestWorld {
                world: context.world(),
            };

            let placement = match place_structure(
                &game_test_world,
                &template,
                test_x,
                test_z,
                test_instance.padding,
            )
            .await
            {
                Ok(placement) => placement,
                Err(error) => {
                    context
                        .source
                        .send_error(TextComponent::text(format!(
                            "Failed to place test instance '{name}': {error}"
                        )))
                        .await;
                    return Ok(0);
                }
            };

            let origin = placement.origin();
            let placed_blocks = template.block_count();

            info!(
                target: "pumpkin::gametest",
                test = name,
                structure = %test_instance.structure,
                origin_x = origin.0.x,
                origin_y = origin.0.y,
                origin_z = origin.0.z,
                placed_blocks,
                nbt = %structure,
                "Loaded GameTest structure"
            );

            context
                .source
                .send_feedback(
                    TextComponent::text(format!(
                        "Placed test instance '{name}' structure '{}' at {} {} {} ({} blocks)",
                        test_instance.structure,
                        origin.0.x,
                        origin.0.y,
                        origin.0.z,
                        placed_blocks
                    )),
                    false,
                )
                .await;

            Ok(1)
        })
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(
        command("test", DESCRIPTION).requires(PERMISSION).then(
            literal("run").then(
                argument(ARG_NAME, StringArgumentType::GreedyPhrase)
                    .suggests(TestInstanceSuggestionProvider)
                    .executes(RunExecutor),
            ),
        ),
    );
}
