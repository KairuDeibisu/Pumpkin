use crate::plugin::{
    PluginMetadata,
    loader::wasm::wasm_host::{
        PluginInitError, PluginInstance, concurrent_store::LegacySyncReentry,
        state::PluginHostState,
    },
};
use pumpkin_host_bindings::v0_2::PluginPre;
use wasmtime::component::{HasSelf, InstancePre, Linker};
use wasmtime::{Engine, Store};

// v0.2 currently backs only GameTest. Other imports are linked as trapping
// stubs from the actual component type at load time; v0.1 remains fully backed.
pub mod gametest;
pub use pumpkin_host_bindings::v0_2::{Plugin, pumpkin};

pub fn add_to_linker(linker: &mut Linker<PluginHostState>) -> wasmtime::Result<()> {
    pumpkin::plugin::gametest::add_to_linker::<_, HasSelf<_>>(linker, |state| state)?;
    // Lifecycle exports transfer an owned context even though its methods are
    // outside this vertical slice. Register its identity and destructor.
    linker.instance("pumpkin:plugin/context@0.2.0")?.resource(
        "context",
        wasmtime::component::ResourceType::host::<pumpkin::plugin::context::Context>(),
        |mut store, rep| {
            store
                .data_mut()
                .resource_table
                .delete::<crate::plugin::loader::wasm::wasm_host::state::ContextResource>(
                wasmtime::component::Resource::new_own(rep),
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

pub fn prepare_plugin(
    instance_pre: &InstancePre<PluginHostState>,
) -> wasmtime::Result<PluginPre<PluginHostState>> {
    PluginPre::new(instance_pre.clone())
}

pub async fn init_plugin(
    engine: &Engine,
    plugin_pre: PluginPre<PluginHostState>,
    legacy_sync_reentry: &LegacySyncReentry,
) -> Result<(PluginInstance, Store<PluginHostState>, PluginMetadata), PluginInitError> {
    let mut store = Store::new(engine, PluginHostState::new());
    store.limiter(|state| &mut state.limits);
    let plugin = legacy_sync_reentry
        .scope_bootstrap(plugin_pre.instantiate_async(&mut store))
        .await
        .map_err(PluginInitError::InstantiationFailed)?;

    store
        .run_concurrent(async |accessor| {
            legacy_sync_reentry
                .scope_bootstrap(plugin.call_init_plugin(accessor))
                .await
        })
        .await
        .map_err(PluginInitError::CallInitPluginFailed)?
        .map_err(PluginInitError::CallInitPluginFailed)?;

    let metadata = store
        .run_concurrent(async |accessor| {
            legacy_sync_reentry
                .scope_bootstrap(plugin.pumpkin_plugin_metadata().call_get_metadata(accessor))
                .await
        })
        .await
        .map_err(PluginInitError::CallGetMetadataFailed)?
        .map_err(PluginInitError::CallGetMetadataFailed)?;

    let metadata = PluginMetadata {
        name: metadata.name,
        version: metadata.version,
        authors: metadata.authors,
        description: metadata.description,
        dependencies: metadata.dependencies,
        permissions: metadata.permissions,
    };

    store
        .data_mut()
        .permissions
        .clone_from(&metadata.permissions);

    Ok((PluginInstance::V0_2(plugin), store, metadata))
}
