#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    pub async fn handle_known_packs(
        &self,
        _config_acknowledged: SKnownPacks<'_>,
        server: &Server,
    ) -> Option<PacketHandlerResult> {
        debug!("Handling known packs");
        // let mut tags_to_send = Vec::new();
        let version = self.version.load();
        if version.supports_configuration_state() {
            self.send_packet(&CFeatureFlags::new(&["minecraft:vanilla".to_string()]))
                .await;
            let registry = Registry::get_synced(version);
            let test_instance_entries = server
                .datapack_manager
                .get_test_instance_registry_entries()
                .await;
            let mut sent_dimension_type = false;
            let mut sent_test_instance = false;
            for reg in &registry {
                if reg.registry_id == "minecraft:dimension_type" {
                    sent_dimension_type = true;
                }
                if reg.registry_id == "minecraft:test_instance" {
                    sent_test_instance = true;
                    self.send_packet(&CRegistryData::new(
                        &reg.registry_id,
                        &test_instance_entries,
                    ))
                    .await;
                    continue;
                }
                self.send_packet(&CRegistryData::new(&reg.registry_id, &reg.registry_entries))
                    .await;
            }
            // The generated synced-registry table can lag behind the current protocol.
            // ResourceSelectorArgument validates /test names against this registry on the
            // client, so always provide it even when the static table omitted it.
            if !sent_test_instance {
                self.send_packet(&CRegistryData::new(
                    &"minecraft:test_instance".to_string(),
                    &test_instance_entries,
                ))
                .await;
            }
            if !sent_dimension_type {
                let dims = [
                    &pumpkin_data::dimension::Dimension::OVERWORLD,
                    &pumpkin_data::dimension::Dimension::OVERWORLD_CAVES,
                    &pumpkin_data::dimension::Dimension::THE_END,
                    &pumpkin_data::dimension::Dimension::THE_NETHER,
                ];
                let dim_entries: Vec<pumpkin_data::registry::RegistryEntryData> = dims
                    .iter()
                    .map(|dim| pumpkin_data::registry::RegistryEntryData {
                        entry_id: dim.minecraft_name.to_string(),
                        data: Some(build_dimension_nbt(dim).into_boxed_slice()),
                    })
                    .collect();
                self.send_packet(&CRegistryData::new(
                    &"minecraft:dimension_type".to_string(),
                    &dim_entries,
                ))
                .await;
            }
        }
        let mut tags = Vec::new();
        for &key in pumpkin_data::tag::RegistryKey::NETWORK_KEYS {
            if pumpkin_data::tag::get_registry_key_tags(version, key)
                .is_some_and(|map| !map.is_empty())
            {
                tags.push(key);
            }
        }
        self.send_packet(&CUpdateTags::new(&tags)).await;

        // We are done with configuring
        self.send_packet(&CFinishConfig).await;

        if !version.supports_configuration_state() {
            return Some(self.handle_config_acknowledged(server).await);
        }

        debug!("Finished config");
        None
    }
}
