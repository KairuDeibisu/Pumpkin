use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, RwLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use pumpkin_data::entity::EntityType;
use pumpkin_util::{
    GameMode,
    math::{vector2::Vector2, vector3::Vector3},
    text::TextComponent,
};

use crate::{
    command::{CommandSender, CommandSource},
    entity::{Entity, EntityBase},
    plugin::Plugin,
    server::Server,
    world::World,
};

#[derive(Clone)]
pub struct GameTestRegistration {
    pub plugin_name: String,
    pub test_class_name: String,
    pub test_name: String,
    pub handler_id: u32,
    plugin: Weak<dyn Plugin>,
}

impl GameTestRegistration {
    #[must_use]
    pub fn id(&self) -> String {
        format!("{}:{}", self.test_class_name, self.test_name)
    }
}

#[derive(Default)]
pub struct GameTestRegistry {
    registrations: RwLock<HashMap<String, GameTestRegistration>>,
}

impl GameTestRegistry {
    fn key(test_class_name: &str, test_name: &str) -> String {
        format!("{test_class_name}:{test_name}").to_ascii_lowercase()
    }

    pub fn register(
        &self,
        plugin_name: String,
        plugin: Weak<dyn Plugin>,
        test_class_name: String,
        test_name: String,
        handler_id: u32,
    ) -> Result<(), String> {
        if test_class_name.trim().is_empty() || test_name.trim().is_empty() {
            return Err("GameTest class and test names must not be empty".to_string());
        }

        let key = Self::key(&test_class_name, &test_name);
        let mut registrations = self
            .registrations
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if let Some(existing) = registrations.get(&key)
            && existing.plugin_name != plugin_name
        {
            return Err(format!(
                "GameTest '{}' is already registered by plugin '{}'",
                existing.id(),
                existing.plugin_name
            ));
        }

        registrations.insert(
            key,
            GameTestRegistration {
                plugin_name,
                test_class_name,
                test_name,
                handler_id,
                plugin,
            },
        );
        Ok(())
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<GameTestRegistration> {
        self.registrations
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&id.to_ascii_lowercase())
            .cloned()
    }

    pub fn remove_plugin(&self, plugin_name: &str) {
        self.registrations
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|_, registration| registration.plugin_name != plugin_name);
    }

    pub async fn run(
        &self,
        server: Arc<Server>,
        world: Arc<World>,
        id: &str,
    ) -> Result<(), String> {
        let registration = self
            .get(id)
            .ok_or_else(|| format!("Unknown plugin GameTest '{id}'"))?;
        let plugin = registration.plugin.upgrade().ok_or_else(|| {
            format!(
                "Plugin '{}' is no longer loaded",
                registration.plugin_name
            )
        })?;

        let test = Arc::new(GameTestContext::new(server, world));
        let result = plugin
            .handle_gametest(registration.handler_id, test.clone())
            .await;
        test.cleanup();
        result
    }
}

pub struct GameTestContext {
    server: Arc<Server>,
    world: Arc<World>,
    simulated_players: Mutex<Vec<Arc<SimulatedPlayer>>>,
}

impl GameTestContext {
    #[must_use]
    pub fn new(server: Arc<Server>, world: Arc<World>) -> Self {
        Self {
            server,
            world,
            simulated_players: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn spawn_simulated_player(
        &self,
        location: Vector3<f64>,
        name: String,
        game_mode: GameMode,
    ) -> Arc<SimulatedPlayer> {
        let player = Arc::new(SimulatedPlayer::new(
            self.server.clone(),
            self.world.clone(),
            location,
            name,
            game_mode,
        ));
        self.simulated_players
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(player.clone());
        player
    }

    pub fn cleanup(&self) {
        let players = self
            .simulated_players
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for player in players.iter() {
            player.disconnect();
        }
    }
}

impl Drop for GameTestContext {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// A deliberately small, non-networked player-like command source.
///
/// It uses a real Pumpkin `Entity` with the player entity type so command
/// selectors and command executors see normal entity position/world state.
/// It is not inserted into the world's connected-player collection and does not
/// emulate a client connection.
pub struct SimulatedPlayer {
    server: Arc<Server>,
    entity: Arc<Entity>,
    name: String,
    game_mode: GameMode,
    disconnected: AtomicBool,
}

impl SimulatedPlayer {
    fn new(
        server: Arc<Server>,
        world: Arc<World>,
        location: Vector3<f64>,
        name: String,
        game_mode: GameMode,
    ) -> Self {
        Self {
            server,
            entity: Arc::new(Entity::new(world, location, &EntityType::PLAYER)),
            name,
            game_mode,
            disconnected: AtomicBool::new(false),
        }
    }

    #[must_use]
    pub fn position(&self) -> Vector3<f64> {
        self.entity.pos.load()
    }

    #[must_use]
    pub const fn game_mode(&self) -> GameMode {
        self.game_mode
    }

    pub fn run_command(&self, command: &str) -> Result<i32, String> {
        if self.disconnected.load(Ordering::Acquire) {
            return Err(format!("Simulated player '{}' is disconnected", self.name));
        }

        let world = self.entity.world.load_full();
        let position = self.entity.pos.load();
        let entity: Arc<dyn EntityBase> = self.entity.clone();
        let source = CommandSource::new(
            CommandSender::Dummy,
            world,
            Some(entity),
            position,
            Vector2::new(self.entity.pitch.load(), self.entity.yaw.load()),
            self.name.clone(),
            TextComponent::text(self.name.clone()),
            self.server.clone(),
        );

        let command = command.trim().trim_start_matches('/');
        if command.is_empty() {
            return Err("Command must not be empty".to_string());
        }

        self.server
            .command_dispatcher
            .load()
            .execute_input(command, &source)
            .map_err(|error| error.to_string())
    }

    pub fn disconnect(&self) {
        self.disconnected.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_disconnected(&self) -> bool {
        self.disconnected.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::GameTestRegistry;

    #[test]
    fn gametest_keys_are_case_insensitive() {
        assert_eq!(
            GameTestRegistry::key("PumpkinTests", "simulatedPlayerTeleport"),
            "pumpkintests:simulatedplayerteleport"
        );
    }
}
