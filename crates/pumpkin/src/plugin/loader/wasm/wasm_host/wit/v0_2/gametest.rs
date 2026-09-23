use std::sync::Arc;

use pumpkin_util::{GameMode as InternalGameMode, math::vector3::Vector3};
use wasmtime::component::{Accessor, HasSelf, Resource};

use crate::plugin::loader::wasm::wasm_host::{
    state::{GameTestResource, PluginHostState, SimulatedPlayerResource},
    wit::v0_2::pumpkin::{
        self,
        plugin::{
            common::{GameMode, Position},
            gametest::{CommandResult, SimulatedPlayer, Test},
        },
    },
};

fn game_mode(mode: Option<GameMode>) -> InternalGameMode {
    match mode.unwrap_or(GameMode::Survival) {
        GameMode::Survival => InternalGameMode::Survival,
        GameMode::Creative => InternalGameMode::Creative,
        GameMode::Adventure => InternalGameMode::Adventure,
        GameMode::Spectator => InternalGameMode::Spectator,
    }
}

impl pumpkin::plugin::gametest::Host for PluginHostState {}

impl pumpkin::plugin::gametest::HostWithStore<PluginHostState> for HasSelf<PluginHostState> {
    async fn register_test(
        accessor: &Accessor<PluginHostState, Self>,
        test_class_name: String,
        test_name: String,
        handler_id: u32,
    ) -> wasmtime::Result<Result<(), String>> {
        accessor.with(|mut host| {
            let state = host.get();
            let plugin = state
                .plugin
                .as_ref()
                .and_then(std::sync::Weak::upgrade)
                .ok_or_else(|| wasmtime::Error::msg("Plugin not found"))?;
            let server = state
                .server
                .as_ref()
                .cloned()
                .ok_or_else(|| wasmtime::Error::msg("Server not found"))?;
            let plugin_name = state
                .name
                .clone()
                .ok_or_else(|| wasmtime::Error::msg("Plugin name not available"))?;

            let plugin: Arc<dyn crate::plugin::Plugin> = plugin;
            Ok(server.gametest_registry.register(
                plugin_name,
                Arc::downgrade(&plugin),
                test_class_name,
                test_name,
                handler_id,
            ))
        })
    }
}

impl pumpkin::plugin::gametest::HostTest for PluginHostState {}

impl pumpkin::plugin::gametest::HostTestWithStore<PluginHostState> for HasSelf<PluginHostState> {
    async fn spawn_simulated_player(
        accessor: &Accessor<PluginHostState, Self>,
        test: Resource<Test>,
        location: Position,
        name: Option<String>,
        mode: Option<GameMode>,
    ) -> wasmtime::Result<Resource<SimulatedPlayer>> {
        accessor.with(|mut host| {
            let state = host.get();
            let context = state
                .resource_table
                .get::<GameTestResource>(&Resource::new_own(test.rep()))
                .map_err(wasmtime::Error::from)?
                .provider
                .clone();

            let player = context.spawn_simulated_player(
                Vector3::new(location.0, location.1, location.2),
                name.unwrap_or_else(|| "Simulated Player".to_string()),
                game_mode(mode),
            );
            state.add_simulated_player(player)
        })
    }

    async fn drop(
        accessor: &Accessor<PluginHostState, Self>,
        rep: Resource<Test>,
    ) -> wasmtime::Result<()> {
        accessor.with(|mut host| {
            let state = host.get();
            if let Ok(resource) = state
                .resource_table
                .get::<GameTestResource>(&Resource::new_own(rep.rep()))
            {
                resource.provider.cleanup();
            }
            let _ = state
                .resource_table
                .delete::<GameTestResource>(Resource::new_own(rep.rep()));
            Ok(())
        })
    }
}

impl pumpkin::plugin::gametest::HostSimulatedPlayer for PluginHostState {}

impl pumpkin::plugin::gametest::HostSimulatedPlayerWithStore<PluginHostState>
    for HasSelf<PluginHostState>
{
    async fn run_command(
        accessor: &Accessor<PluginHostState, Self>,
        player: Resource<SimulatedPlayer>,
        command: String,
    ) -> wasmtime::Result<Result<CommandResult, String>> {
        accessor.with(|mut host| {
            let state = host.get();
            let player = state
                .resource_table
                .get::<SimulatedPlayerResource>(&Resource::new_own(player.rep()))
                .map_err(wasmtime::Error::from)?
                .provider
                .clone();

            Ok(player
                .run_command(&command)
                .map(|success_count| CommandResult { success_count }))
        })
    }

    async fn get_position(
        accessor: &Accessor<PluginHostState, Self>,
        player: Resource<SimulatedPlayer>,
    ) -> wasmtime::Result<Position> {
        accessor.with(|mut host| {
            let state = host.get();
            let position = state
                .resource_table
                .get::<SimulatedPlayerResource>(&Resource::new_own(player.rep()))
                .map_err(wasmtime::Error::from)?
                .provider
                .position();
            Ok((position.x, position.y, position.z))
        })
    }

    async fn disconnect(
        accessor: &Accessor<PluginHostState, Self>,
        player: Resource<SimulatedPlayer>,
    ) -> wasmtime::Result<()> {
        accessor.with(|mut host| {
            let state = host.get();
            state
                .resource_table
                .get::<SimulatedPlayerResource>(&Resource::new_own(player.rep()))
                .map_err(wasmtime::Error::from)?
                .provider
                .disconnect();
            Ok(())
        })
    }

    async fn drop(
        accessor: &Accessor<PluginHostState, Self>,
        rep: Resource<SimulatedPlayer>,
    ) -> wasmtime::Result<()> {
        accessor.with(|mut host| {
            let state = host.get();
            if let Ok(resource) = state
                .resource_table
                .get::<SimulatedPlayerResource>(&Resource::new_own(rep.rep()))
            {
                resource.provider.disconnect();
            }
            let _ = state
                .resource_table
                .delete::<SimulatedPlayerResource>(Resource::new_own(rep.rep()));
            Ok(())
        })
    }
}
