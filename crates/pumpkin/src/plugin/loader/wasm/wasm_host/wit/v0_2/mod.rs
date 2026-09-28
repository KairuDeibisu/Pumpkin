use std::{
    collections::HashMap,
    num::NonZeroUsize,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use pumpkin_gametest::{GameTestError, GameTestExecution, GameTestFunction, GameTestResult};
use pumpkin_scheduler::{
    ExecutionDomain, ExecutorFuture, GlobalScheduler, SchedulerConfig, SchedulerError,
    TaskExecutor, TaskRequest,
};
use wasmtime::{
    Engine, Store,
    component::{HasSelf, InstancePre, Linker, Resource},
};

use crate::{
    plugin::{
        PluginMetadata,
        loader::wasm::wasm_host::{
            PluginInitError, PluginInstance, WasmPlugin,
            concurrent_store::LegacySyncReentry,
            state::{FromResource, PluginHostState},
        },
    },
    server::Server,
};

use pumpkin::plugin::gametest;

const MAXIMUM_TASKS: NonZeroUsize = NonZeroUsize::new(1024).unwrap();
const TURNS_PER_POLL: NonZeroUsize = NonZeroUsize::new(64).unwrap();
const SLOW_TURN_THRESHOLD: Duration = Duration::from_millis(50);
pub use pumpkin_host_bindings::v0_2::{Plugin, PluginPre, pumpkin};

impl FromResource for gametest::Test {
    type Internal = Arc<GameTestExecution>;
}

impl gametest::Host for PluginHostState {
    async fn register(
        &mut self,
        class: String,
        name: String,
        handler_id: u32,
    ) -> wasmtime::Result<Result<(), String>> {
        let Some(plugin) = self.plugin.as_ref().and_then(Weak::upgrade) else {
            return Ok(Err("Plugin not found".to_owned()));
        };
        let Some(server) = &self.server else {
            return Ok(Err("Server not found".to_owned()));
        };
        Ok(server
            .game_test_functions
            .register(server, &plugin, &class, &name, handler_id))
    }
}

impl gametest::HostTest for PluginHostState {
    async fn succeed(
        &mut self,
        test: Resource<gametest::Test>,
    ) -> wasmtime::Result<Result<(), String>> {
        Ok(self.get(&test)?.succeed())
    }

    async fn drop(&mut self, test: Resource<gametest::Test>) -> wasmtime::Result<()> {
        self.drop(test)
    }
}

#[derive(Default)]
pub struct GameTestFunctions {
    state: Mutex<RegistryState>,
}

#[derive(Default)]
struct RegistryState {
    functions: HashMap<String, (Weak<WasmPlugin>, u32)>,
    scheduler: Option<GlobalScheduler>,
    driver: Option<tokio::task::AbortHandle>,
}

struct ServerExecutor {
    runtime: tokio::runtime::Handle,
    tasks: tokio_util::task::TaskTracker,
    driver: Mutex<Option<tokio::task::AbortHandle>>,
}

impl TaskExecutor for ServerExecutor {
    fn spawn(&self, future: ExecutorFuture) -> Result<(), SchedulerError> {
        let driver = self.tasks.spawn_on(future, &self.runtime);
        *self
            .driver
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(driver.abort_handle());
        Ok(())
    }
}

impl GameTestFunctions {
    fn register(
        &self,
        server: &Server,
        plugin: &Arc<WasmPlugin>,
        class: &str,
        name: &str,
        handler_id: u32,
    ) -> Result<(), String> {
        let id = format!("{class}:{name}");
        if class.is_empty()
            || name.is_empty()
            || !pumpkin_util::identifier::Identifier::is_valid_namespace(class)
            || !pumpkin_util::identifier::Identifier::is_valid_path(name)
        {
            return Err(format!("Invalid GameTest function identifier '{id}'"));
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .functions
            .get(&id)
            .is_some_and(|(owner, _)| owner.strong_count() > 0)
        {
            return Err(format!("GameTest function '{id}' is already registered"));
        }
        if state.scheduler.is_none() {
            // Bound admission while allowing callbacks to suspend without holding the game tick.
            let config = SchedulerConfig::new(MAXIMUM_TASKS, TURNS_PER_POLL, SLOW_TURN_THRESHOLD);
            let executor = ServerExecutor {
                runtime: server.runtime.clone(),
                tasks: server.tasks.clone(),
                driver: Mutex::new(None),
            };
            state.scheduler =
                Some(GlobalScheduler::start(config, &executor).map_err(|error| error.to_string())?);
            state.driver = executor
                .driver
                .into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state
            .functions
            .insert(id, (Arc::downgrade(plugin), handler_id));
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn GameTestFunction>> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (plugin, handler_id) = state.functions.get(id)?;
        Some(Arc::new(WasmGameTestFunction {
            plugin: plugin.clone(),
            handler_id: *handler_id,
            scheduler: state.scheduler.clone()?,
        }))
    }

    pub fn remove_plugin(&self, plugin: &WasmPlugin) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .functions
            .retain(|_, (owner, _)| !std::ptr::eq(owner.as_ptr(), plugin));
    }

    pub fn shutdown(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.functions.clear();
        if let Some(driver) = state.driver.take() {
            driver.abort();
        }
        state.scheduler = None;
    }
}

struct WasmGameTestFunction {
    plugin: Weak<WasmPlugin>,
    handler_id: u32,
    scheduler: GlobalScheduler,
}

impl GameTestFunction for WasmGameTestFunction {
    fn start(&self, execution: Arc<GameTestExecution>) -> GameTestResult<()> {
        let plugin = self
            .plugin
            .upgrade()
            .ok_or_else(|| GameTestError::World("GameTest plugin is unloaded".to_owned()))?;
        let handler_id = self.handler_id;
        let task = self
            .scheduler
            .submit(TaskRequest::new(
                ExecutionDomain::Global,
                move |_| async move {
                    if !execution.is_active() {
                        return Ok(());
                    }
                    if let Err(error) = plugin.handle_game_test(handler_id, execution.clone()).await
                    {
                        execution.fail(error.to_string());
                    }
                    Ok(())
                },
            ))
            .map_err(|error| GameTestError::World(error.to_string()))?;
        drop(task);
        Ok(())
    }
}

pub fn add_to_linker(linker: &mut Linker<PluginHostState>) -> wasmtime::Result<()> {
    Plugin::add_to_linker::<_, HasSelf<_>>(linker, |state: &mut PluginHostState| state)
}

pub fn prepare_plugin(
    instance: &InstancePre<PluginHostState>,
) -> wasmtime::Result<PluginPre<PluginHostState>> {
    PluginPre::new(instance.clone())
}

pub async fn init_plugin(
    engine: &Engine,
    pre: PluginPre<PluginHostState>,
    policy: &LegacySyncReentry,
) -> Result<(PluginInstance, Store<PluginHostState>, PluginMetadata), PluginInitError> {
    let mut store = Store::new(engine, PluginHostState::new());
    store.limiter(|state| &mut state.limits);
    let plugin = policy
        .scope_bootstrap(pre.instantiate_async(&mut store))
        .await
        .map_err(PluginInitError::InstantiationFailed)?;
    store
        .run_concurrent(async |accessor| {
            policy
                .scope_bootstrap(plugin.call_init_plugin(accessor))
                .await
        })
        .await
        .map_err(PluginInitError::CallInitPluginFailed)?
        .map_err(PluginInitError::CallInitPluginFailed)?;
    let metadata = store
        .run_concurrent(async |accessor| {
            policy
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
