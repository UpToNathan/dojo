//! Original CUA private-worker transport without SDK or platform dependencies.
mod environment;
mod options;
mod process;
pub mod worker;
pub use options::*;
use serde_json::Value;
use std::sync::Arc;
pub use worker::ActionCompletion;

#[derive(Clone)]
pub struct WorkerRetirement(process::SharedChild);
impl WorkerRetirement {
    pub fn retire(&self) -> std::io::Result<()> {
        self.0.retire()
    }
    pub fn pid(&self) -> u32 {
        self.0.id()
    }
}

pub struct OwnedWorker {
    client: Arc<worker::PrivateWorkerClient>,
    retirement: WorkerRetirement,
}

impl OwnedWorker {
    pub fn spawn(
        options: WorkerOptions,
        on_spawn: impl FnOnce(WorkerRetirement),
    ) -> Result<Self, DriverError> {
        let options = worker::validate_worker_options(
            options.binary_path,
            options.host_bundle_id,
            options.startup_timeout_ms,
            options.shutdown_timeout_ms,
            options.configured_driver,
            options.environment,
            options.inherit_stderr,
        )?;
        let mut retirement = None;
        let client = worker::PrivateWorkerClient::spawn(options, |handle| {
            retirement = Some(handle.clone());
            on_spawn(handle);
        })?;
        Ok(Self {
            client,
            retirement: retirement.unwrap(),
        })
    }
    pub fn retirement_handle(&self) -> WorkerRetirement {
        self.retirement.clone()
    }
    pub fn is_available(&self) -> bool {
        self.client.is_available()
    }
    pub fn metadata(
        &self,
    ) -> impl std::future::Future<Output = Result<DriverMetadata, DriverError>> + Send + 'static
    {
        let client = self.client.clone();
        async move { client.metadata().await }
    }
    pub fn list_tools(
        &self,
    ) -> impl std::future::Future<Output = Result<Value, DriverError>> + Send + 'static {
        let client = self.client.clone();
        async move { client.list_tools().await }
    }
    pub fn invoke(
        &self,
        name: String,
        arguments: Value,
        session_handle: Option<String>,
    ) -> impl std::future::Future<Output = Result<Value, DriverError>> + Send + 'static {
        let client = self.client.clone();
        async move { client.invoke(&name, arguments, session_handle).await }
    }
    pub fn invoke_blocking(
        &self,
        name: String,
        arguments: Value,
        session_handle: Option<String>,
    ) -> Result<Value, DriverError> {
        self.client
            .request_sync("call", Some(name), Some(arguments), session_handle)
    }
    pub fn list_tools_blocking(&self) -> Result<Value, DriverError> {
        self.client.request_sync("list", None, None, None)
    }
    pub fn bind_session(&self, options: TrustedSessionOptions) -> Result<String, DriverError> {
        self.client.bind_session(options)
    }
    pub fn close_session(&self, handle: &str) {
        self.client.close_session(handle);
    }
    pub fn list_sessions(
        &self,
    ) -> impl std::future::Future<Output = Result<Value, DriverError>> + Send + 'static {
        let client = self.client.clone();
        async move { client.list_host_sessions().await }
    }
    pub fn shutdown(
        &self,
    ) -> impl std::future::Future<Output = Result<(), DriverError>> + Send + 'static {
        let client = self.client.clone();
        async move { client.shutdown().await }
    }
}
impl Drop for OwnedWorker {
    fn drop(&mut self) {
        let _ = self.retirement.retire();
    }
}
