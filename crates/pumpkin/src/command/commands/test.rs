use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
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

            info!(
                target: "pumpkin::gametest",
                test = name,
                structure = %test_instance.structure,
                nbt = %structure,
                "Loaded GameTest structure"
            );

            context
                .source
                .send_feedback(
                    TextComponent::text(format!(
                        "Loaded test instance '{name}' structure '{}' (parsed NBT written to the server log)",
                        test_instance.structure
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
                argument(ARG_NAME, StringArgumentType::SingleWord)
                    .suggests(TestInstanceSuggestionProvider)
                    .executes(RunExecutor),
            ),
        ),
    );
}
