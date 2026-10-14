//! Dojo's bounded local perception executor. Capture publication remains session owned.
use crate::perception_client::{
    ExpectedExtensionIdentity, PerceptionCancellation, PerceptionClient, PerceptionWorkerConfig,
    containment::ContainmentLimits,
};
use anyhow::{Context, Result, ensure};
use cua_driver_contract::{
    ParseVisualRegionsInput, ParseVisualRegionsOutput, VisualActionCoordinateSpace,
    VisualCaptureProvenance, VisualCaptureSource, VisualScreenshotReference,
};
use dojo_computer::PerceptionRead;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::os::unix::fs::MetadataExt;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const BASELINE: &str = "88aff7e5424e9ed45b93ded85eb4111472ca5190";
const MANIFEST: &str = "59f5197c98d0bbe20668993771a19fa495624027e69ef87a75c6606986a60b13";
const RUNTIME: &str = "5bd5bedf736fc501692435d0ec4f6e8b2bdf48cd30af8e6d00d61b3ddc9a7ab8";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Installation {
    source_commit: String,
    executable: PathBuf,
    executable_sha256: String,
    manifest: PathBuf,
    runtime: PathBuf,
    interpreter: PathBuf,
    libraries: Vec<PathBuf>,
    loader_search_paths: Vec<PathBuf>,
}

pub struct CapturePerception {
    config: PerceptionWorkerConfig,
    busy: AtomicBool,
    quarantined: AtomicBool,
}

impl CapturePerception {
    /// Host-selected immutable installation. Opening starts no worker.
    pub fn open(installation: &Path) -> Result<Self> {
        let installation = immutable_file(installation)?;
        ensure!(
            fs::metadata(&installation)?.len() <= 16_384,
            "perception installation too large"
        );
        let mut config: Installation = serde_json::from_slice(&fs::read(installation)?)?;
        ensure!(
            config.source_commit == BASELINE,
            "perception worker source baseline differs"
        );
        config.executable = checked_digest(&config.executable, &config.executable_sha256)?;
        config.manifest = checked_digest(&config.manifest, MANIFEST)?;
        config.runtime = checked_digest(&config.runtime, RUNTIME)?;
        config.interpreter = immutable_file(&config.interpreter)?;
        ensure!(
            config.libraries.len() <= 64 && config.loader_search_paths.len() <= 16,
            "perception loader closure too large"
        );
        let libraries = config
            .libraries
            .iter()
            .map(|file| immutable_file(file))
            .collect::<Result<Vec<_>>>()?;
        for directory in &config.loader_search_paths {
            let canonical = immutable_path(directory)?;
            ensure!(
                canonical.is_dir()
                    && libraries
                        .iter()
                        .any(|file| file.parent() == Some(canonical.as_path())),
                "loader directory has no reviewed member"
            );
        }
        ensure!(
            elf_interpreter(&fs::read(&config.executable)?)? == config.interpreter,
            "worker interpreter differs from reviewed closure"
        );
        let limits = ContainmentLimits {
            allow_system_runtime_trees: false,
            additional_readable_files: libraries,
            additional_executable_paths: vec![config.interpreter],
            loader_search_paths: config.loader_search_paths,
            ..ContainmentLimits::default()
        };
        let mut worker =
            PerceptionWorkerConfig::new(config.executable).with_host_containment(limits);
        worker.args = vec![
            "--manifest".into(),
            config.manifest.to_string_lossy().into_owned(),
            "--onnx-runtime-library".into(),
            config.runtime.to_string_lossy().into_owned(),
            "--extension-id".into(),
            "cua-perception".into(),
            "--extension-version".into(),
            "0.2.1".into(),
        ];
        worker.expected_extension_identity = Some(ExpectedExtensionIdentity {
            id: "cua-perception".into(),
            version: "0.2.1".into(),
        });
        worker.max_frame_bytes = 16 * 1024 * 1024;
        PerceptionClient::new(worker.clone()).map_err(|error| anyhow::anyhow!("{error:?}"))?;
        Ok(Self {
            config: worker,
            busy: AtomicBool::new(false),
            quarantined: AtomicBool::new(false),
        })
    }

    /// One cold parse of a ticket minted by CaptureSession. Completion must also
    /// pass the original session's authoritative complete_perception_read gate.
    pub async fn parse(
        &self,
        read: &PerceptionRead,
        input: ParseVisualRegionsInput,
        cancelled: Arc<AtomicBool>,
    ) -> Result<ParseVisualRegionsOutput> {
        input
            .validate()
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        ensure!(
            !cancelled.load(Ordering::Acquire),
            "perception request cancelled"
        );
        ensure!(
            input.capture_id == read.capture_id(),
            "perception input refers to another capture"
        );
        ensure!(
            !self.quarantined.load(Ordering::Acquire),
            "perception cleanup unsettled; restart backend"
        );
        ensure!(
            self.busy
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "perception backend busy"
        );
        let _permit = Permit(&self.busy);
        let remaining = read.deadline().saturating_duration_since(Instant::now());
        let budget = remaining
            .checked_sub(Duration::from_secs(5))
            .context("capture has insufficient perception lifetime")?;
        let (width, height) = read.dimensions();
        let image = read.png();
        ensure!(
            width <= 8192
                && height <= 8192
                && u64::from(width) * u64::from(height) <= 32 * 1024 * 1024
                && image.len() <= 8 * 1024 * 1024,
            "capture exceeds worker limits"
        );
        let window = read
            .frame()
            .window
            .as_ref()
            .context("perception currently requires an exact window capture")?;
        let source = VisualCaptureSource::Window {
            pid: window.pid.try_into()?,
            window_id: window.window_id,
        };
        let mut config = self.config.clone();
        config.request_timeout = budget.min(Duration::from_secs(25));
        let client = PerceptionClient::new(config).map_err(|error| anyhow::anyhow!("{error:?}"))?;
        let token = PerceptionCancellation::default();
        let monitor = async {
            loop {
                if cancelled.load(Ordering::Acquire) {
                    token.cancel();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        let started = Instant::now();
        let request = client.parse(&input.capture_id, width, height, &image, &token);
        tokio::pin!(request);
        let result = tokio::select! {
            result = &mut request => result,
            _ = monitor => request.await,
        };
        if !client.cleanup_settled() {
            self.quarantined.store(true, Ordering::Release);
        }
        let worker = result.map_err(|error| anyhow::anyhow!("{error:?}"))?;
        ensure!(
            !cancelled.load(Ordering::Acquire) && Instant::now() < read.deadline(),
            "perception completion cancelled or expired"
        );
        validate_echo(read, &worker)?;
        let frame = read.frame();
        let provenance = VisualCaptureProvenance {
            capture_id: read.capture_id(),
            source,
            screenshot: VisualScreenshotReference {
                reference: read.digest().into(),
                width,
                height,
                mime_type: "image/png".into(),
                sha256: Some(read.digest().into()),
            },
            action_coordinate_space: VisualActionCoordinateSpace::Affine {
                m11: f64::from(frame.logical_width) / f64::from(width),
                m12: 0.,
                m21: 0.,
                m22: f64::from(frame.logical_height) / f64::from(height),
                tx: f64::from(frame.logical_x),
                ty: f64::from(frame.logical_y),
            },
            captured_at: None,
        };
        let output = crate::perception_tools::build_local_output(
            &input,
            provenance,
            worker,
            started.elapsed().as_millis(),
        )
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        ensure!(
            output.regions.iter().all(|region| !region.interactive),
            "perception returned interactive regions"
        );
        Ok(output)
    }
}

struct Permit<'a>(&'a AtomicBool);
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn validate_echo(read: &PerceptionRead, worker: &serde_json::Value) -> Result<()> {
    let (width, height) = read.dimensions();
    ensure!(
        worker["capture_id"].as_str() == Some(read.capture_id().as_str())
            && worker["image"]["sha256"].as_str() == Some(read.digest())
            && worker["image"]["width"].as_u64() == Some(width.into())
            && worker["image"]["height"].as_u64() == Some(height.into()),
        "worker omitted or changed capture identity"
    );
    ensure!(
        worker["runtime"] == "onnx_runtime_cpu"
            && worker["identity"]["model"]["manifest_sha256"] == MANIFEST
            && worker["identity"]["onnx_runtime"]["library_sha256"] == RUNTIME,
        "worker artifact identity differs"
    );
    Ok(())
}

fn immutable_path(path: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;
    let relative = canonical
        .strip_prefix("/nix/store")
        .context("perception installation must be in Nix store")?;
    let mut components = relative.components();
    let member = components
        .next()
        .context("Nix store itself is not an installation")?;
    ensure!(
        member.as_os_str().to_string_lossy().contains('-'),
        "invalid Nix member"
    );
    let mut current = PathBuf::from("/nix/store").join(member.as_os_str());
    loop {
        let metadata = fs::symlink_metadata(&current)?;
        ensure!(
            metadata.uid() == 0
                && metadata.mode() & 0o222 == 0
                && !metadata.file_type().is_symlink(),
            "perception member is mutable"
        );
        let Some(component) = components.next() else {
            break;
        };
        current.push(component.as_os_str());
    }
    Ok(canonical)
}
fn immutable_file(path: &Path) -> Result<PathBuf> {
    let path = immutable_path(path)?;
    ensure!(path.is_file(), "perception member is not a file");
    Ok(path)
}
fn checked_digest(path: &Path, digest: &str) -> Result<PathBuf> {
    let path = immutable_file(path)?;
    ensure!(
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid pinned digest"
    );
    ensure!(
        fs::metadata(&path)?.len() <= 256 * 1024 * 1024,
        "perception executable/artifact too large"
    );
    ensure!(
        format!("{:x}", Sha256::digest(fs::read(&path)?)) == digest,
        "perception member digest differs"
    );
    Ok(path)
}
fn elf_interpreter(bytes: &[u8]) -> Result<PathBuf> {
    ensure!(
        bytes.len() >= 64 && &bytes[..6] == b"\x7fELF\x02\x01",
        "expected Linux ELF64 worker"
    );
    let offset = usize::try_from(u64::from_le_bytes(bytes[32..40].try_into()?))?;
    let size = usize::from(u16::from_le_bytes(bytes[54..56].try_into()?));
    let count = usize::from(u16::from_le_bytes(bytes[56..58].try_into()?));
    ensure!(size >= 56 && count <= 1024, "invalid ELF program table");
    for index in 0..count {
        let start = offset
            .checked_add(index.checked_mul(size).context("ELF overflow")?)
            .context("ELF overflow")?;
        let header = bytes
            .get(start..start.checked_add(56).context("ELF overflow")?)
            .context("ELF program table truncated")?;
        if u32::from_le_bytes(header[..4].try_into()?) != 3 {
            continue;
        }
        let offset = usize::try_from(u64::from_le_bytes(header[8..16].try_into()?))?;
        let size = usize::try_from(u64::from_le_bytes(header[32..40].try_into()?))?;
        ensure!(size > 1 && size <= 4096, "invalid ELF interpreter");
        let value = bytes
            .get(offset..offset.checked_add(size).context("ELF overflow")?)
            .context("ELF interpreter truncated")?;
        ensure!(
            value.last() == Some(&0) && !value[..size - 1].contains(&0),
            "invalid ELF interpreter path"
        );
        return Ok(PathBuf::from(std::str::from_utf8(&value[..size - 1])?));
    }
    anyhow::bail!("worker has no ELF interpreter")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dojo_computer::{
        CaptureAuthority, CaptureFrame, CaptureScope, CompositorIdentity, OutputFrame,
        WindowIdentity,
    };
    use serde_json::json;

    fn read() -> (dojo_computer::CaptureSession, PerceptionRead) {
        let png = include_bytes!(
            "../../cua-perception/tests/quality-corpus/images/native-controls-light.png"
        )
        .to_vec();
        let authority = Arc::new(CaptureAuthority::new().unwrap());
        let session = authority.session("test-perception".into()).unwrap();
        let frame = CaptureFrame {
            output: OutputFrame {
                name: "fixture".into(),
                x: 0,
                y: 0,
                width: 960,
                height: 600,
                scale: 1.,
                transform: "Normal".into(),
                compositor: Some(CompositorIdentity {
                    pid: std::process::id(),
                    process_start_time: 1,
                }),
            },
            logical_x: 0,
            logical_y: 0,
            logical_width: 960,
            logical_height: 600,
            window: Some(WindowIdentity {
                window_id: 42,
                pid: std::process::id(),
                process_start_time: 1,
                app_id: Some("fixture".into()),
                workspace_id: 1,
                rendered_rect: (0., 0., 960., 600.),
            }),
            coordination_revision: 1,
        };
        let observation = session
            .publish(
                CaptureScope {
                    output: "fixture".into(),
                    region: None,
                    window_id: Some(42),
                    scale: None,
                },
                frame,
                png,
                960,
                600,
            )
            .unwrap();
        let read = session
            .begin_perception_read(&observation.capture_id)
            .unwrap();
        (session, read)
    }

    #[test]
    fn mutable_installations_and_malformed_elf_are_refused() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        assert!(CapturePerception::open(temp.path()).is_err());
        for bytes in [vec![], vec![0; 64], b"#!/bin/sh\n".to_vec()] {
            assert!(elf_interpreter(&bytes).is_err());
        }
        let mut bytes = vec![0; 64];
        bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
        bytes[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
        bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
        bytes[56..58].copy_from_slice(&1u16.to_le_bytes());
        assert!(elf_interpreter(&bytes).is_err());
    }

    #[test]
    fn worker_echo_is_required_for_all_capture_and_artifact_fields() {
        let (_session, read) = read();
        let expected = json!({"capture_id": read.capture_id(), "image": {"sha256": read.digest(), "width": 960, "height": 600}, "runtime": "onnx_runtime_cpu", "identity": {"model": {"manifest_sha256": MANIFEST}, "onnx_runtime": {"library_sha256": RUNTIME}}});
        validate_echo(&read, &expected).unwrap();
        for pointer in [
            "/capture_id",
            "/image/sha256",
            "/image/width",
            "/image/height",
            "/runtime",
            "/identity/model/manifest_sha256",
            "/identity/onnx_runtime/library_sha256",
        ] {
            let mut bad = expected.clone();
            *bad.pointer_mut(pointer).unwrap() = serde_json::Value::Null;
            assert!(validate_echo(&read, &bad).is_err(), "{pointer}");
        }
    }

    #[tokio::test]
    #[ignore = "requires immutable staged real artifacts via DOJO_PERCEPTION_INSTALLATION"]
    async fn contained_real_parse_and_cancellation_preserve_capture() {
        let installation = std::env::var_os("DOJO_PERCEPTION_INSTALLATION")
            .expect("staged immutable installation");
        let backend = Arc::new(CapturePerception::open(Path::new(&installation)).unwrap());
        let (session, read) = read();
        let input = ParseVisualRegionsInput {
            capture_id: read.capture_id(),
            options: Default::default(),
        };
        let output = backend
            .parse(&read, input.clone(), Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        assert_eq!(
            output.capture.screenshot.sha256.as_deref(),
            Some(read.digest())
        );
        assert!(
            output
                .regions
                .iter()
                .any(|region| region.text.as_deref() == Some("SETTINGS"))
        );
        assert!(output.regions.iter().all(|region| !region.interactive));
        session.complete_perception_read(&read).unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            signal.store(true, Ordering::Release);
        });
        let started = Instant::now();
        assert!(backend.parse(&read, input, cancelled).await.is_err());
        assert!(started.elapsed() < Duration::from_secs(6));
        assert!(!backend.busy.load(Ordering::Acquire));
        assert!(!backend.quarantined.load(Ordering::Acquire));
        session.complete_perception_read(&read).unwrap();
        for entry in fs::read_dir("/proc").unwrap().flatten() {
            let Ok(exe) = fs::read_link(entry.path().join("exe")) else {
                continue;
            };
            if exe != backend.config.executable {
                continue;
            }
            let status = fs::read_to_string(entry.path().join("status")).unwrap_or_default();
            assert!(
                !status
                    .lines()
                    .any(|line| line.trim() == format!("PPid:\t{}", std::process::id())),
                "owned worker was not reaped"
            );
        }
    }
}
