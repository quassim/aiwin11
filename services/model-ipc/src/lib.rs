#[cfg(windows)]
mod broker_host;
#[cfg(windows)]
mod pipe;
#[cfg(windows)]
mod task_host;
#[cfg(windows)]
mod worker;

#[cfg(windows)]
pub use broker_host::dispatch as broker_dispatch;

#[cfg(windows)]
pub use task_host::{TaskCommand, TaskPresence, TaskReplay, TaskSupervisor};

#[cfg(windows)]
pub use worker::{
    initialize_core, journal_page, list_lm_studio_models, list_loaded_models, list_local_models, propose_cloud,
    propose_local, route_local_model, run_worker,
};
