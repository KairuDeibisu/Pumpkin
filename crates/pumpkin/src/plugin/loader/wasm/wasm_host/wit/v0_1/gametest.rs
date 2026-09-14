use std::sync::Arc;

use pumpkin_data::entity::EntityType;
use pumpkin_gametest::SimulatedPlayerController;
use pumpkin_protocol::java::client::play::{
    CPlayerInfoUpdate, CSetEntityMetadata, Player as JavaPlayer, PlayerAction, PlayerInfoFlags,
};
use pumpkin_util::{math::vector3::Vector3, version::JavaMinecraftVersion};
use uuid::Uuid;
use wasmtime::component::{Access, HasSelf, Resource};

use crate::{
    entity::{Entity, EntityBase, living::LivingEntity},
    net::{ClientPlatform, java::JavaClient},
    plugin::loader::wasm::wasm_host::{
        state::{PluginHostState, WorldResource},
        wit::v0_1::pumpkin::plugin::{
            common::Position,
            gametest::{self, SimulatedPlayer},
            world::World,
        },
    },
    server::Server,
    world::World as InternalWorld,
};

/// Server-side player actor used by GameTest.
///
/// It deliberately has no network client. Movement is driven by the shared
/// `pumpkin-gametest` controller and applied through normal entity collision handling.
pub struct GameTestSimulatedPlayer {
    living_entity: LivingEntity,
    controller: SimulatedPlayerController,
}

impl GameTestSimulatedPlayer {
    fn new(world: Arc<InternalWorld>, position: Vector3<f64>) -> Arc<Self> {
        Arc::new(Self {
            living_entity: LivingEntity::new(Entity::from_uuid(
                Uuid::new_v4(),
                world,
                position,
                &EntityType::PLAYER,
            )),
            controller: SimulatedPlayerController::new(),
        })
    }

    fn move_to_location(&self, position: Vector3<f64>) {
        self.controller.move_to_location(position);
    }
}

impl EntityBase for GameTestSimulatedPlayer {
    fn tick(&self, caller: &dyn EntityBase, server: &Server) {
        if let Some(motion) = self
            .controller
            .next_step(self.living_entity.entity.pos.load())
        {
            self.living_entity.entity.move_entity(caller, motion);
            self.controller
                .finish_step(self.living_entity.entity.pos.load());
        }

        self.living_entity.tick(caller, server);
    }

    fn send_java_spawn_packet(&self, client: &JavaClient) {
        let entity = &self.living_entity.entity;
        let name = format!("GameTest{}", entity.entity_id);
        let actions = [
            PlayerAction::AddPlayer {
                name: &name,
                properties: &[],
            },
            PlayerAction::UpdateListed(false),
        ];
        let players = [JavaPlayer {
            uuid: entity.entity_uuid,
            actions: &actions,
        }];
        let player_info = CPlayerInfoUpdate::new(
            (PlayerInfoFlags::ADD_PLAYER | PlayerInfoFlags::UPDATE_LISTED).bits(),
            &players,
        );
        if let Ok(data) = client.serialize_packet(&player_info) {
            client.try_enqueue_packet(data);
        }

        let version = client.version.load();
        let is_mob = entity.entity_type.mob || self.get_mob().is_some();
        let metadata = self.java_spawn_metadata(version);
        if version < JavaMinecraftVersion::V_1_19 && is_mob {
            let spawn_packet = entity.create_spawn_living_packet(metadata.clone());
            if let Ok(data) = client.serialize_packet(&spawn_packet) {
                client.try_enqueue_packet(data);
            }
            if version >= JavaMinecraftVersion::V_1_15
                && let Some(meta) = metadata
            {
                let meta_packet = CSetEntityMetadata::new(entity.entity_id.into(), meta);
                if let Ok(meta_data) = client.serialize_packet(&meta_packet) {
                    client.try_enqueue_packet(meta_data);
                }
            }
        } else {
            let spawn_packet = entity.create_spawn_packet();
            if let Ok(data) = client.serialize_packet(&spawn_packet) {
                client.try_enqueue_packet(data);
            }
            if let Some(meta) = metadata
                && (version >= JavaMinecraftVersion::V_1_9 || meta.last().copied() == Some(127))
            {
                let meta_packet = CSetEntityMetadata::new(entity.entity_id.into(), meta);
                if let Ok(meta_data) = client.serialize_packet(&meta_packet) {
                    client.try_enqueue_packet(meta_data);
                }
            }
        }
    }

    fn get_entity(&self) -> &Entity {
        &self.living_entity.entity
    }

    fn get_living_entity(&self) -> Option<&LivingEntity> {
        Some(&self.living_entity)
    }

    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
}

struct SimulatedPlayerResource {
    provider: Arc<GameTestSimulatedPlayer>,
}

fn simulated_player_from_resource(
    state: &PluginHostState,
    resource: &Resource<SimulatedPlayer>,
) -> wasmtime::Result<Arc<GameTestSimulatedPlayer>> {
    state
        .resource_table
        .get::<SimulatedPlayerResource>(&Resource::new_own(resource.rep()))
        .map_err(|_| wasmtime::Error::msg("invalid simulated player resource handle"))
        .map(|resource| Arc::clone(&resource.provider))
}

fn active_plugin(
    state: &PluginHostState,
) -> wasmtime::Result<Arc<crate::plugin::loader::wasm::wasm_host::WasmPlugin>> {
    state
        .plugin
        .as_ref()
        .and_then(std::sync::Weak::upgrade)
        .ok_or_else(|| wasmtime::Error::msg("Plugin instance not available"))
}

impl gametest::Host for PluginHostState {}

impl gametest::HostSimulatedPlayer for PluginHostState {
    async fn move_to_location(
        &mut self,
        player: Resource<SimulatedPlayer>,
        location: Position,
    ) -> wasmtime::Result<()> {
        let player = simulated_player_from_resource(self, &player)?;
        player.move_to_location(Vector3::new(location.0, location.1, location.2));
        Ok(())
    }

    async fn drop(&mut self, rep: Resource<SimulatedPlayer>) -> wasmtime::Result<()> {
        self.resource_table
            .delete::<SimulatedPlayerResource>(Resource::new_own(rep.rep()))
            .map_err(wasmtime::Error::from)?;
        Ok(())
    }
}

impl gametest::HostWithStore<PluginHostState> for HasSelf<PluginHostState> {
    async fn create_simulated_player(
        mut host: Access<'_, PluginHostState, Self>,
        world: Resource<World>,
        location: Position,
    ) -> wasmtime::Result<Resource<SimulatedPlayer>> {
        let (world, plugin) = {
            let state = host.get();
            let world = state
                .resource_table
                .get::<WorldResource>(&Resource::new_own(world.rep()))
                .map_err(|_| wasmtime::Error::msg("invalid world resource handle"))?
                .provider
                .clone();
            (world, active_plugin(state)?)
        };

        let player = GameTestSimulatedPlayer::new(
            Arc::clone(&world),
            Vector3::new(location.0, location.1, location.2),
        );
        let entity: Arc<dyn EntityBase> = player.clone();
        plugin
            .store
            .pump_blocking(&mut host, move || world.spawn_entity(entity))
            .await?;

        let resource = host
            .get()
            .resource_table
            .push(SimulatedPlayerResource { provider: player })?;
        Ok(Resource::new_own(resource.rep()))
    }
}
