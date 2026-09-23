use std::sync::Arc;

use pumpkin_util::{GameMode as InternalGameMode, math::vector3::Vector3};
use wasmtime::component::Resource;

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

impl pumpkin::plugin::gametest::Host for PluginHostState {
    async fn register_test(
        &mut self,
        test_class_name: String,
        test_name: String,
        handler_id: u32,
    ) -> wasmtime::Result<Result<(), String>> {
        let plugin = self
            .plugin
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| wasmtime::Error::msg("Plugin not found"))?;
        let server = self
            .server
            .as_ref()
            .cloned()
            .ok_or_else(|| wasmtime::Error::msg("Server not found"))?;
        let plugin_name = self
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
    }
}

impl pumpkin::plugin::gametest::HostTest for PluginHostState {
    async fn spawn_simulated_player(
        &mut self,
        test: Resource<Test>,
        location: Position,
        name: Option<String>,
        mode: Option<GameMode>,
    ) -> wasmtime::Result<Resource<SimulatedPlayer>> {
        let context = self
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
        self.add_simulated_player(player)
    }

    async fn drop(&mut self, rep: Resource<Test>) -> wasmtime::Result<()> {
        if let Ok(resource) = self
            .resource_table
            .get::<GameTestResource>(&Resource::new_own(rep.rep()))
        {
            resource.provider.cleanup();
        }
        let _ = self
            .resource_table
            .delete::<GameTestResource>(Resource::new_own(rep.rep()));
        Ok(())
    }
}

impl pumpkin::plugin::gametest::HostSimulatedPlayer for PluginHostState {
    async fn run_command(
        &mut self,
        player: Resource<SimulatedPlayer>,
        command: String,
    ) -> wasmtime::Result<Result<CommandResult, String>> {
        let player = self
            .resource_table
            .get::<SimulatedPlayerResource>(&Resource::new_own(player.rep()))
            .map_err(wasmtime::Error::from)?
            .provider
            .clone();

        Ok(player
            .run_command(&command)
            .map(|success_count| CommandResult { success_count }))
    }

    async fn get_position(
        &mut self,
        player: Resource<SimulatedPlayer>,
    ) -> wasmtime::Result<Position> {
        let position = self
            .resource_table
            .get::<SimulatedPlayerResource>(&Resource::new_own(player.rep()))
            .map_err(wasmtime::Error::from)?
            .provider
            .position();
        Ok((position.x, position.y, position.z))
    }

    async fn disconnect(
        &mut self,
        player: Resource<SimulatedPlayer>,
    ) -> wasmtime::Result<()> {
        self.resource_table
            .get::<SimulatedPlayerResource>(&Resource::new_own(player.rep()))
            .map_err(wasmtime::Error::from)?
            .provider
            .disconnect();
        Ok(())
    }

    async fn drop(&mut self, rep: Resource<SimulatedPlayer>) -> wasmtime::Result<()> {
        if let Ok(resource) = self
            .resource_table
            .get::<SimulatedPlayerResource>(&Resource::new_own(rep.rep()))
        {
            resource.provider.disconnect();
        }
        let _ = self
            .resource_table
            .delete::<SimulatedPlayerResource>(Resource::new_own(rep.rep()));
        Ok(())
    }
}
