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
        let plugin = registration
            .plugin
            .upgrade()
            .ok_or_else(|| format!("Plugin '{}' is no longer loaded", registration.plugin_name))?;

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

        let source = self.command_source();

        let command = command.trim().trim_start_matches('/');
        if command.is_empty() {
            return Err("Command must not be empty".to_string());
        }

        self.server
            .command_dispatcher
            .load()
            .execute_input(command, &source)
            .map_err(|error| error.message.get_text())
    }

    fn command_source(&self) -> CommandSource {
        let world = self.entity.world.load_full();
        let position = self.entity.pos.load();
        let entity: Arc<dyn EntityBase> = self.entity.clone();
        CommandSource::new(
            CommandSender::Dummy,
            world,
            Some(entity),
            position,
            Vector2::new(self.entity.pitch.load(), self.entity.yaw.load()),
            self.name.clone(),
            TextComponent::text(self.name.clone()),
            self.server.clone(),
        )
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
    use super::*;

    struct TestPlugin;
    impl Plugin for TestPlugin {}

    #[test]
    fn gametest_registration_retains_identity_and_rejects_other_plugins() {
        let registry = GameTestRegistry::default();
        let plugin: Arc<dyn Plugin> = Arc::new(TestPlugin);
        let weak = Arc::downgrade(&plugin);
        registry
            .register(
                "owner".into(),
                weak.clone(),
                "Suite".into(),
                "Test".into(),
                42,
            )
            .unwrap();
        let registration = registry.get("SUITE:test").unwrap();
        assert_eq!(registration.plugin_name, "owner");
        assert_eq!(registration.id(), "Suite:Test");
        assert_eq!(registration.handler_id, 42);
        assert!(Weak::ptr_eq(&registration.plugin, &weak));
        assert!(
            registry
                .register(
                    "other".into(),
                    weak.clone(),
                    "suite".into(),
                    "test".into(),
                    9
                )
                .is_err()
        );
        assert_eq!(registry.get("suite:test").unwrap().handler_id, 42);
        // The owning plugin can replace its own registration.
        registry
            .register("owner".into(), weak, "Suite".into(), "Test".into(), 43)
            .unwrap();
        assert_eq!(registry.get("suite:test").unwrap().handler_id, 43);
        registry.remove_plugin("other");
        assert!(registry.get("suite:test").is_some());
        registry.remove_plugin("owner");
        assert!(registry.get("suite:test").is_none());
        // Registrations must not keep unloaded plugins alive.
        drop(plugin);
        assert!(registration.plugin.upgrade().is_none());
    }

    #[test]
    fn gametest_keys_are_case_insensitive() {
        assert_eq!(
            GameTestRegistry::key("PumpkinTests", "simulatedPlayerTeleport"),
            "pumpkintests:simulatedplayerteleport"
        );
    }

    struct CallbackPlugin {
        players: Mutex<Vec<Arc<SimulatedPlayer>>>,
        fail: bool,
    }

    impl Plugin for CallbackPlugin {
        fn handle_gametest(
            &self,
            handler_id: u32,
            test: Arc<GameTestContext>,
        ) -> crate::plugin::PluginFuture<'_, Result<(), String>> {
            Box::pin(async move {
                assert_eq!(handler_id, 73);
                let player = test.spawn_simulated_player(
                    Vector3::new(0.0, 80.0, 0.0),
                    "PumpkinBot".into(),
                    GameMode::Survival,
                );
                assert_eq!(player.position(), Vector3::new(0.0, 80.0, 0.0));
                let source = player.command_source();
                assert!(Arc::ptr_eq(
                    source.entity.as_ref().unwrap(),
                    &(player.entity.clone() as Arc<dyn EntityBase>)
                ));
                // This exercises the parser, @s selector, teleport executor and EntityBase::teleport.
                assert!(player.run_command("tp @s 10 80 10").unwrap() > 0);
                assert_eq!(player.position(), Vector3::new(10.0, 80.0, 10.0));
                self.players.lock().unwrap().push(player);
                if self.fail {
                    Err("intentional guest failure".into())
                } else {
                    Ok(())
                }
            })
        }
    }

    async fn test_server() -> (tempfile::TempDir, Arc<Server>) {
        use pumpkin_config::{AdvancedConfiguration, BasicConfiguration};
        let directory = tempfile::tempdir().unwrap();
        let basic = BasicConfiguration {
            default_level_name: directory
                .path()
                .join("world")
                .to_string_lossy()
                .into_owned(),
            allow_nether: false,
            allow_end: false,
            allow_chat_reports: false,
            ..Default::default()
        };
        let mut advanced = AdvancedConfiguration::default();
        advanced.networking.bedrock.online_mode = false;
        let data = crate::data::VanillaData {
            banned_ip_list: RwLock::new(Default::default()),
            banned_player_list: RwLock::new(Default::default()),
            operator_config: RwLock::new(Default::default()),
            user_cache: RwLock::new(Default::default()),
            whitelist_config: RwLock::new(Default::default()),
        };
        let server = Server::new(basic, advanced, Default::default(), data).await;
        (directory, server)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn gametest_command_dispatch_and_cleanup_after_success_and_failure() {
        let (_directory, server) = test_server().await;
        let world = server.worlds.load()[0].clone();
        for fail in [false, true] {
            let callback = Arc::new(CallbackPlugin {
                players: Mutex::new(Vec::new()),
                fail,
            });
            let plugin: Arc<dyn Plugin> = callback.clone();
            server
                .gametest_registry
                .register(
                    "test".into(),
                    Arc::downgrade(&plugin),
                    "Suite".into(),
                    "Teleport".into(),
                    73,
                )
                .unwrap();
            let result = server
                .gametest_registry
                .run(server.clone(), world.clone(), "suite:teleport")
                .await;
            assert_eq!(
                result,
                if fail {
                    Err("intentional guest failure".into())
                } else {
                    Ok(())
                }
            );
            let players = callback.players.lock().unwrap();
            assert_eq!(players.len(), 1);
            let player = &players[0];
            assert!(player.is_disconnected());
            player.disconnect();
            player.disconnect();
            assert!(player.is_disconnected());
            assert!(
                player
                    .run_command("tp @s 0 80 0")
                    .unwrap_err()
                    .contains("disconnected")
            );
        }
    }

    /// Run with PUMPKIN_GAMETEST_COMPONENT pointing to the built TS example.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires the built TypeScript GameTest component"]
    async fn gametest_component_round_trip() {
        use crate::plugin::{
            Context,
            loader::{PluginLoader, wasm::WasmPluginLoader},
        };
        let path = std::env::var("PUMPKIN_GAMETEST_COMPONENT").expect("component path");
        let (_directory, server) = test_server().await;
        let loader = WasmPluginLoader::new(false);
        let (plugin, metadata, _data) = loader
            .load(std::path::Path::new(&path))
            .await
            .expect("load v0.2 component");
        let name = metadata.name.clone();
        let context = Arc::new(Context::new(
            metadata,
            server.clone(),
            Default::default(),
            server.plugin_manager.clone(),
            Default::default(),
        ));
        plugin
            .on_load(context.clone())
            .await
            .expect("register during on-load");
        let registration = server
            .gametest_registry
            .get("pumpkintests:simulatedplayerteleport")
            .expect("guest registration");
        assert_eq!(registration.plugin_name, name);
        let world = server.worlds.load()[0].clone();
        let result = server
            .gametest_registry
            .run(server.clone(), world, &registration.id())
            .await;
        if let Ok(expected) = std::env::var("PUMPKIN_GAMETEST_EXPECT_ERROR") {
            assert!(result.unwrap_err().contains(&expected));
        } else {
            result.expect("guest teleport assertion");
        }
        plugin.on_unload(context).await.expect("unload guest");
        assert!(server.gametest_registry.get(&registration.id()).is_none());

        if let Ok(path) = std::env::var("PUMPKIN_V01_COMPONENT") {
            let (_plugin, metadata, _) = loader
                .load(std::path::Path::new(&path))
                .await
                .expect("load v0.1 component");
            assert!(!metadata.name.is_empty());
        }
    }
}
