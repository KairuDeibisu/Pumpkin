use pumpkin_protocol::java::client::play::{
    ArgumentType, CommandSuggestion, StringProtoArgBehavior, SuggestionProviders,
};
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
use tracing::info;

use crate::command::args::{
    Arg, ArgumentConsumer, ConsumeResult, ConsumedArgs, FindArg, GetClientSideArgParser,
    SuggestResult,
};
use crate::command::node::dispatcher::CommandDispatcher as LegacyCommandDispatcher;
use crate::command::tree::builder::{argument, literal};
use crate::command::tree::{CommandTree, RawArgs};
use crate::command::{CommandError, CommandExecutor, CommandResult, CommandSender};
use crate::server::Server;
use crate::server::ticker::{GameTestRequest, enqueue_game_test};

const NAMES: [&str; 1] = ["test"];
const DESCRIPTION: &str = "Runs a GameTest test instance.";
const PERMISSION: &str = "minecraft:command.test";
const ARG_NAME: &str = "name";
const TEST_POS_Z_OFFSET_FROM_PLAYER: i32 = 3;

struct TestInstanceArgumentConsumer;

impl GetClientSideArgParser for TestInstanceArgumentConsumer {
    fn get_client_side_parser(&self) -> ArgumentType {
        ArgumentType::String(StringProtoArgBehavior::SingleWord)
    }

    fn get_client_side_suggestion_type_override(&self) -> Option<SuggestionProviders> {
        Some(SuggestionProviders::AskServer)
    }
}

impl ArgumentConsumer for TestInstanceArgumentConsumer {
    fn consume<'a>(
        &'a self,
        _sender: &'a CommandSender,
        _server: &'a Server,
        args: &mut RawArgs<'a>,
    ) -> ConsumeResult<'a> {
        let value = args.pop().map(|arg| arg.value);
        Box::pin(async move { value.map(Arg::Simple) })
    }

    fn suggest<'a>(
        &'a self,
        _sender: &CommandSender,
        server: &'a Server,
        _input: &'a str,
    ) -> SuggestResult<'a> {
        Box::pin(async move {
            let suggestions = server
                .datapack_manager
                .get_test_instance_names()
                .await
                .into_iter()
                .map(|name| CommandSuggestion::new(name, None))
                .collect();
            Ok(Some(suggestions))
        })
    }
}

impl<'a> FindArg<'a> for TestInstanceArgumentConsumer {
    type Data = &'a str;

    fn find_arg(args: &'a ConsumedArgs, name: &str) -> Result<Self::Data, CommandError> {
        match args.get(name) {
            Some(Arg::Simple(value)) => Ok(value),
            _ => Err(CommandError::InvalidConsumption(Some(name.to_string()))),
        }
    }
}

struct RunExecutor;

impl CommandExecutor for RunExecutor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        server: &'a Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let name = TestInstanceArgumentConsumer::find_arg(args, ARG_NAME)?;
            let world = sender
                .world_or_first(server)
                .ok_or(CommandError::InvalidRequirement)?;
            let (test_x, test_z) = if let Some(source_pos) = sender.position() {
                (
                    source_pos.x.floor() as i32,
                    source_pos.z.floor() as i32 + TEST_POS_Z_OFFSET_FROM_PLAYER,
                )
            } else {
                let level_info = world.level_info.load();
                (
                    level_info.spawn_x,
                    level_info.spawn_z + TEST_POS_Z_OFFSET_FROM_PLAYER,
                )
            };

            enqueue_game_test(GameTestRequest::new(name, world, test_x, test_z)).await;

            info!(
                target: "pumpkin::gametest",
                test = name,
                test_x,
                test_z,
                "Queued GameTest request"
            );

            sender
                .send_message(TextComponent::text(format!(
                    "Queued test instance '{name}'"
                )))
                .await;

            Ok(1)
        })
    }
}

pub fn init_command_tree() -> CommandTree {
    CommandTree::new(NAMES, DESCRIPTION).then(
        literal("run").then(
            argument(ARG_NAME, TestInstanceArgumentConsumer).execute(RunExecutor),
        ),
    )
}

pub fn register(dispatcher: &mut LegacyCommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher
        .fallback_dispatcher
        .register(init_command_tree(), PERMISSION);
}
