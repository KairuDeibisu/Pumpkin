use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::core::string::StringArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};

const DESCRIPTION: &str = "Runs plugin-registered GameTests.";
const PERMISSION: &str = "pumpkin:command.gametest";

struct RunExecutor;

impl CommandExecutor for RunExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let test_id = StringArgumentType::get(context, "test")?.to_string();
        let server = context.source.server().clone();
        let world = context.world().clone();
        let sender = context.source.output.clone();

        if server.gametest_registry.get(&test_id).is_none() {
            sender.send_message(TextComponent::text(format!(
                "Unknown plugin GameTest '{test_id}'"
            )));
            return Ok(0);
        }

        sender.send_message(TextComponent::text(format!(
            "Running plugin GameTest '{test_id}'..."
        )));

        let run_server = server.clone();
        server.spawn_task(async move {
            let result = run_server
                .gametest_registry
                .run(run_server.clone(), world, &test_id)
                .await;
            match result {
                Ok(()) => {
                    sender.send_message(TextComponent::text(format!("GameTest '{test_id}' passed")))
                }
                Err(error) => sender.send_message(TextComponent::text(format!(
                    "GameTest '{test_id}' failed: {error}"
                ))),
            }
        });

        Ok(1)
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(command("gametest", DESCRIPTION).requires(PERMISSION).then(
        literal("run").then(argument("test", StringArgumentType::SingleWord).executes(RunExecutor)),
    ));
}
