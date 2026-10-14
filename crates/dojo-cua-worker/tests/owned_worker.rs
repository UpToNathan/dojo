#![cfg(feature = "test-fixture")]
use dojo_cua_worker::*;
use serde_json::json;
use std::time::{Duration, Instant};

fn options() -> WorkerOptions {
    WorkerOptions {
        binary_path: env!("CARGO_BIN_EXE_dojo-cua-worker-fixture").into(),
        host_bundle_id: "fixture".into(),
        startup_timeout_ms: Some(2000),
        shutdown_timeout_ms: Some(2000),
        inherit_stderr: true,
        environment: vec![],
        configured_driver: ConfiguredDriverOptions {
            claude_code_compatibility: false,
            authorization: RuntimeAuthorizationOptions {
                allowed_modes: vec![SessionPermissionMode::Standard],
                compatibility_mode: SessionPermissionMode::Standard,
                compatibility_capability_manifest_path: None,
                compatibility_bounded_manifest_path: None,
                unrestricted_acknowledged: false,
                max_session_ttl_seconds: 3600,
                max_idle_ttl_seconds: 300,
            },
        },
    }
}

async fn wait_started(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "fixture child never started its blocking call"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_retirement_interrupts_request_and_preserves_other_worker() {
    let worker = OwnedWorker::spawn(options(), |_| {}).unwrap();
    let other = OwnedWorker::spawn(options(), |_| {}).unwrap();
    let retirement = worker.retirement_handle();
    assert_ne!(retirement.pid(), other.retirement_handle().pid());
    let directory = tempfile::tempdir().unwrap();
    let ready = directory.path().join("started");
    let request = tokio::spawn(worker.invoke("block".into(), json!({"ready_file":ready}), None));
    wait_started(&ready).await;
    let began = Instant::now();
    retirement.retire().unwrap();
    let error = tokio::time::timeout(Duration::from_secs(2), request)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(matches!(
        error,
        DriverError::ActionInterrupted {
            completion: ActionCompletion::Unknown,
            ..
        }
    ));
    assert!(began.elapsed() < Duration::from_secs(2));
    assert!(!worker.is_available());
    assert!(other.is_available());
    assert_eq!(
        other.metadata().await.unwrap().pid,
        other.retirement_handle().pid()
    );
    retirement.retire().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_drop_retires_child_even_when_request_future_retains_transport() {
    let worker = OwnedWorker::spawn(options(), |_| {}).unwrap();
    let retirement = worker.retirement_handle();
    let directory = tempfile::tempdir().unwrap();
    let ready = directory.path().join("started");
    let request = tokio::spawn(worker.invoke("block".into(), json!({"ready_file":ready}), None));
    wait_started(&ready).await;
    drop(worker);
    let error = tokio::time::timeout(Duration::from_secs(2), request)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(matches!(
        error,
        DriverError::ActionInterrupted {
            completion: ActionCompletion::Unknown,
            ..
        }
    ));
    retirement.retire().unwrap();
}

#[test]
fn retirement_is_registered_before_startup_readiness_wait() {
    let result = OwnedWorker::spawn(options(), |retirement| retirement.retire().unwrap());
    assert!(matches!(
        result,
        Err(DriverError::ActionInterrupted {
            completion: ActionCompletion::NotStarted,
            ..
        })
    ));
}

#[tokio::test]
async fn copied_session_and_call_protocol_preserves_results() {
    let worker = OwnedWorker::spawn(options(), |_| {}).unwrap();
    assert_eq!(worker.list_tools().await.unwrap(), json!({"tools":[]}));
    let handle = worker
        .bind_session(TrustedSessionOptions {
            public_session: "fixture-session".into(),
            mode: SessionPermissionMode::Standard,
            ttl_seconds: 60,
            idle_ttl_seconds: 30,
            capability_manifest_path: None,
            bounded_manifest_path: None,
        })
        .unwrap();
    assert_eq!(handle, "fixture-session");
    let result = worker
        .invoke("echo".into(), json!({"x":20}), Some(handle.clone()))
        .await
        .unwrap();
    assert_eq!(
        result,
        json!({"content":[],"isError":false,"structuredContent":{"echo":{"x":20}}})
    );
    worker.close_session(&handle);
    worker.shutdown().await.unwrap();
}
