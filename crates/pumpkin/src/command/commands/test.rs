use pumpkin_data::{Block, BlockStateId};
use pumpkin_nbt::NbtCompound;
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

const DESCRIPTION: &str = "Runs a GameTest test instance.";
const PERMISSION: &str = "minecraft:command.test";
const ARG_NAME: &str = "name";
const TEST_POS_Z_OFFSET_FROM_PLAYER: i32 = 3;
const STRUCTURE_OFFSET: [i32; 3] = [0, 1, 1];

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

fn read_vec3(compound: &NbtCompound, name: &str) -> Result<[i32; 3], String> {
    let values = compound
        .get_list(name)
        .ok_or_else(|| format!("Structure is missing '{name}' int list"))?;
    let [x, y, z] = values else {
        return Err(format!("Structure '{name}' must contain exactly three integers"));
    };

    Ok([
        x.extract_int()
            .ok_or_else(|| format!("Structure '{name}' contains a non-integer value"))?,
        y.extract_int()
            .ok_or_else(|| format!("Structure '{name}' contains a non-integer value"))?,
        z.extract_int()
            .ok_or_else(|| format!("Structure '{name}' contains a non-integer value"))?,
    ])
}

fn resolve_palette(structure: &NbtCompound) -> Result<Vec<BlockStateId>, String> {
    let palette = structure
        .get_list("palette")
        .ok_or_else(|| "Structure is missing 'palette'".to_string())?;
    let mut states = Vec::with_capacity(palette.len());

    for (index, entry) in palette.iter().enumerate() {
        let entry = entry
            .extract_compound()
            .ok_or_else(|| format!("Palette entry {index} is not a compound"))?;
        let name = entry
            .get_string("Name")
            .ok_or_else(|| format!("Palette entry {index} is missing 'Name'"))?;
        let block = Block::from_name(name)
            .ok_or_else(|| format!("Unknown block '{name}' in structure palette"))?;

        let state = if let Some(properties) = entry.get_compound("Properties") {
            let mut property_pairs = Vec::with_capacity(properties.child_tags.len());
            for (property_name, property_value) in &properties.child_tags {
                let property_value = property_value.extract_string().ok_or_else(|| {
                    format!(
                        "Block '{name}' property '{property_name}' in palette entry {index} is not a string"
                    )
                })?;
                property_pairs.push((property_name.as_ref(), property_value));
            }

            block.state_from_properties(&property_pairs).ok_or_else(|| {
                format!(
                    "No Pumpkin block state matches palette entry {index} for '{name}'"
                )
            })?
        } else {
            block.default_state
        };

        states.push(state.id);
    }

    Ok(states)
}

async fn manifest_structure(
    context: &CommandContext<'_>,
    structure: &NbtCompound,
    padding: i32,
) -> Result<(BlockPos, usize), String> {
    let size = read_vec3(structure, "size")?;
    if size.iter().any(|axis| *axis <= 0) {
        return Err(format!("Structure has invalid size {size:?}"));
    }

    let palette = resolve_palette(structure)?;
    let blocks = structure
        .get_list("blocks")
        .ok_or_else(|| "Structure is missing 'blocks'".to_string())?;
    let mut placements = Vec::with_capacity(blocks.len());

    // Validate the complete structure before changing the world so malformed NBT
    // cannot leave a half-placed test behind.
    for (index, block) in blocks.iter().enumerate() {
        let block = block
            .extract_compound()
            .ok_or_else(|| format!("Structure block {index} is not a compound"))?;
        let pos = read_vec3(block, "pos")?;
        if pos[0] < 0
            || pos[1] < 0
            || pos[2] < 0
            || pos[0] >= size[0]
            || pos[1] >= size[1]
            || pos[2] >= size[2]
        {
            return Err(format!(
                "Structure block {index} position {pos:?} is outside size {size:?}"
            ));
        }

        let state_index = block
            .get_int("state")
            .ok_or_else(|| format!("Structure block {index} is missing integer 'state'"))?;
        let state_index = usize::try_from(state_index)
            .map_err(|_| format!("Structure block {index} has negative state index"))?;
        let state = palette.get(state_index).copied().ok_or_else(|| {
            format!("Structure block {index} references missing palette state {state_index}")
        })?;
        placements.push((pos, state));
    }

    let world = context.world();
    let source_pos = &context.source.position;
    let test_x = source_pos.x.floor() as i32;
    let test_z = source_pos.z.floor() as i32 + TEST_POS_Z_OFFSET_FROM_PLAYER;
    // Pumpkin's heightmap accessor returns the top occupied Y; vanilla's
    // getHeightmapPos uses the first free Y above it.
    let test_y = world
        .get_heightmap_height_async(ChunkHeightmapType::WorldSurface, test_x, test_z)
        .await
        + 1;

    // Vanilla's TestInstanceBlockEntity places the structure at
    // testBlockPos + padding + (0, 1, 1). We do not create the test-instance
    // block yet, but retain the same structure origin.
    let origin = BlockPos::new(
        test_x + padding + STRUCTURE_OFFSET[0],
        test_y + padding + STRUCTURE_OFFSET[1],
        test_z + padding + STRUCTURE_OFFSET[2],
    );

    let clear_flags = BlockFlags::NOTIFY_LISTENERS
        | BlockFlags::SKIP_DROPS
        | BlockFlags::SKIP_REDSTONE_WIRE_STATE_REPLACEMENT
        | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK;
    for x in 0..size[0] {
        for y in 0..size[1] {
            for z in 0..size[2] {
                let pos = BlockPos::new(origin.0.x + x, origin.0.y + y, origin.0.z + z);
                world
                    .set_block_state(&pos, Block::AIR.default_state.id, clear_flags)
                    .await;
            }
        }
    }

    // StructureTemplate::placeInWorld uses listener updates while suppressing
    // immediate shape/redstone callbacks. These are Pumpkin's closest matching
    // flags and keep command/test/redstone blocks inert while the template appears.
    let place_flags = BlockFlags::NOTIFY_LISTENERS
        | BlockFlags::MOVED
        | BlockFlags::SKIP_REDSTONE_WIRE_STATE_REPLACEMENT
        | BlockFlags::SKIP_BLOCK_ADDED_CALLBACK;
    for (relative, state) in &placements {
        let pos = BlockPos::new(
            origin.0.x + relative[0],
            origin.0.y + relative[1],
            origin.0.z + relative[2],
        );
        world.set_block_state(&pos, *state, place_flags).await;
    }

    Ok((origin, placements.len()))
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

            let (origin, placed_blocks) =
                match manifest_structure(context, &structure, test_instance.padding).await {
                    Ok(result) => result,
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
                        origin.0.x, origin.0.y, origin.0.z,
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
