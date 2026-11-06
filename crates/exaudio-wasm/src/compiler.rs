//! Untrusted compilation runs in a killable, memory-limited helper process.
//! Only artifacts received from this exact helper invocation may be deserialized.
use crate::Limits;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use wasmtime::{Config, Engine, component::Component};

const MAGIC: &[u8; 8] = b"EXAUDIO1";
const HEADER: usize = 56;

pub(crate) fn engine_config(stack: usize) -> Config {
    let mut config = Config::new();
    config
        .wasm_component_model(true)
        .consume_fuel(true)
        .epoch_interruption(true)
        .max_wasm_stack(stack);
    config
}
fn helper_path(limits: &Limits) -> Result<PathBuf> {
    if let Some(path) = &limits.compiler_path {
        ensure!(
            path.is_absolute(),
            "compiler helper override must be an absolute trusted path"
        );
        return Ok(path.clone());
    }
    let executable = std::env::current_exe()?;
    let mut directory = executable.parent().context("executable has no parent")?;
    // Rust's unit-test executables live in target/<profile>/deps; the explicitly
    // built helper is in target/<profile>. Never search PATH or the working dir.
    if cfg!(test) && directory.file_name().is_some_and(|name| name == "deps") {
        directory = directory.parent().context("test profile directory")?;
    }
    Ok(directory.join(format!(
        "exaudio-wasm-worker{}",
        std::env::consts::EXE_SUFFIX
    )))
}

pub(crate) fn compile(
    engine: &Engine,
    bytes: &[u8],
    limits: &Limits,
    cancel: &Arc<AtomicBool>,
) -> Result<Component> {
    compile_inner(engine, bytes, limits, cancel, |_| {})
}
fn compile_inner(
    engine: &Engine,
    bytes: &[u8],
    limits: &Limits,
    cancel: &Arc<AtomicBool>,
    started: impl FnOnce(u32),
) -> Result<Component> {
    let deadline = Instant::now()
        .checked_add(limits.compile_timeout)
        .context("invalid compile timeout")?;
    ensure!(!cancel.load(Ordering::Relaxed), "compilation cancelled");
    ensure!(
        bytes.len() <= limits.component_bytes,
        "component size limit exceeded"
    );
    let identity: [u8; 32] = Sha256::digest(bytes).into();
    let directory = tempfile::Builder::new()
        .prefix("exaudio-compile-")
        .tempdir()?;
    let source = directory.path().join("source.wasm");
    std::fs::write(&source, bytes)?;
    let mut command = Command::new(helper_path(limits)?);
    command
        .arg("--compile")
        .arg(&source)
        .arg(limits.wasm_stack_bytes.to_string())
        .arg(limits.component_bytes.to_string())
        .arg(limits.artifact_bytes.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_child_limits(&mut command, limits.compiler_memory_bytes)?;
    let child = command
        .spawn()
        .context("starting required exaudio-wasm-worker sibling executable")?;
    let mut child = ChildGuard(child);
    // Windows assignment precedes the start byte; the helper cannot compile or
    // initialize Wasmtime until its OS memory/job limit is installed.
    let _job = attach_child_limits(&child.0, limits.compiler_memory_bytes)?;
    started(child.0.id());
    ensure!(!cancel.load(Ordering::Relaxed), "compilation cancelled");
    child
        .0
        .stdin
        .take()
        .context("compiler stdin")?
        .write_all(&[1])?;
    let stdout = child.0.stdout.take().context("compiler stdout")?;
    let stderr = child.0.stderr.take().context("compiler stderr")?;
    let cap = limits
        .artifact_bytes
        .checked_add(HEADER)
        .context("artifact limit overflow")?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        let result = stdout
            .take(cap as u64 + 1)
            .read_to_end(&mut output)
            .map_err(anyhow::Error::from)
            .and_then(|_| {
                ensure!(
                    output.len() <= cap,
                    "compiler artifact output limit exceeded"
                );
                Ok(output)
            });
        let _ = sender.send(result);
    });
    let errors = thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stderr.take(4096).read_to_end(&mut output);
        output
    });
    let mut output = None;
    let outcome = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err(anyhow::anyhow!("compilation cancelled"));
        }
        if Instant::now() >= deadline {
            break Err(anyhow::anyhow!("compiler deadline exceeded"));
        }
        if output.is_none() {
            match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(Ok(bytes)) => output = Some(bytes),
                Ok(Err(error)) => break Err(error),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err(anyhow::anyhow!("compiler output reader disconnected"));
                }
            }
        } else {
            // Wait on child termination, bounded by cancellation/deadline checks.
            thread::park_timeout(Duration::from_millis(10));
        }
        match child.0.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    break Err(anyhow::anyhow!("compiler exited with {status}"));
                }
                if let Some(bytes) = output.take() {
                    break Ok(bytes);
                }
            }
            Ok(None) => {}
            Err(error) => break Err(error.into()),
        }
    };
    // Kill/wait is unconditional on every error path; no timed-out compiler keeps
    // consuming CPU or holds a temp file after its runtime generation is dropped.
    if outcome.is_err() {
        let _ = child.0.kill();
    }
    let _ = child.0.wait();
    let _ = reader.join();
    let errors = errors.join().unwrap_or_default();
    let output = outcome
        .with_context(|| format!("compiler diagnostics: {}", String::from_utf8_lossy(&errors)))?;
    ensure!(
        output.len() >= HEADER && &output[..8] == MAGIC,
        "invalid compiler receipt"
    );
    ensure!(
        output[8..40] == identity,
        "compiler source identity mismatch"
    );
    let stack = u64::from_le_bytes(output[40..48].try_into()?);
    let length = u64::from_le_bytes(output[48..56].try_into()?);
    ensure!(
        stack == limits.wasm_stack_bytes as u64 && length == (output.len() - HEADER) as u64,
        "compiler config/artifact identity mismatch"
    );
    ensure!(
        !cancel.load(Ordering::Relaxed) && Instant::now() < deadline,
        "compilation cancelled or expired"
    );
    // SAFETY: these bytes come exclusively from our trusted, installed helper's
    // Engine::precompile_component for the supplied source and exact config. The
    // temporary input directory is private, no guest runs in the helper, and no
    // external precompiled artifacts or user-selected cache files are accepted.
    // Wasmtime additionally verifies serialization version/config compatibility.
    unsafe { Component::deserialize(engine, &output[HEADER..]) }
        .context("loading trusted compiler artifact")
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(windows)]
fn apply_child_limits(command: &mut Command, _: usize) -> Result<()> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
    command.creation_flags(CREATE_NO_WINDOW);
    Ok(())
}
#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(windows)]
fn attach_child_limits(child: &Child, bytes: usize) -> Result<Job> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::HANDLE,
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, SetInformationJobObject,
        },
    };
    unsafe {
        let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        ensure!(
            !handle.is_null(),
            "CreateJobObjectW: {}",
            std::io::Error::last_os_error()
        );
        let job = Job(handle);
        let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        information.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        information.ProcessMemoryLimit = bytes;
        ensure!(
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &information as *const _ as *const _,
                std::mem::size_of_val(&information) as u32
            ) != 0,
            "setting compiler job limit: {}",
            std::io::Error::last_os_error()
        );
        ensure!(
            AssignProcessToJobObject(handle, child.as_raw_handle() as HANDLE) != 0,
            "assigning compiler job: {}",
            std::io::Error::last_os_error()
        );
        Ok(job)
    }
}
#[cfg(unix)]
fn apply_child_limits(command: &mut Command, bytes: usize) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let bytes = libc::rlim_t::try_from(bytes).context("compiler memory limit overflow")?;
    unsafe {
        command.pre_exec(move || {
            let limit = libc::rlimit {
                rlim_cur: bytes,
                rlim_max: bytes,
            };
            if libc::setrlimit(libc::RLIMIT_AS, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let core = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::setrlimit(libc::RLIMIT_CORE, &core) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}
#[cfg(unix)]
fn attach_child_limits(_: &Child, _: usize) -> Result<()> {
    Ok(())
}
#[cfg(not(any(windows, unix)))]
fn apply_child_limits(_: &mut Command, _: usize) -> Result<()> {
    anyhow::bail!("compiler process memory isolation unavailable on this platform")
}
#[cfg(not(any(windows, unix)))]
fn attach_child_limits(_: &Child, _: usize) -> Result<()> {
    anyhow::bail!("compiler process isolation unavailable on this platform")
}

/// Entry point used only by the installed `exaudio-wasm-worker` executable.
#[doc(hidden)]
pub fn worker_main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    ensure!(
        args.next().as_deref() == Some(std::ffi::OsStr::new("--compile")),
        "expected --compile"
    );
    let source = PathBuf::from(args.next().context("missing source")?);
    let parse = |value: Option<std::ffi::OsString>| -> Result<usize> {
        value
            .context("missing worker limit")?
            .to_str()
            .context("invalid worker limit")?
            .parse()
            .context("invalid worker limit")
    };
    let stack = parse(args.next())?;
    let input_limit = parse(args.next())?;
    let artifact_limit = parse(args.next())?;
    ensure!(args.next().is_none(), "unexpected worker arguments");
    let mut gate = [0];
    std::io::stdin().read_exact(&mut gate)?;
    ensure!(gate == [1], "invalid compiler start gate");
    let mut bytes = Vec::new();
    std::fs::File::open(source)?
        .take(input_limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= input_limit, "compiler input limit exceeded");
    let identity = Sha256::digest(&bytes);
    let engine = Engine::new(&engine_config(stack))?;
    let artifact = engine.precompile_component(&bytes)?;
    ensure!(
        artifact.len() <= artifact_limit,
        "compiled artifact limit exceeded"
    );
    let mut output = std::io::stdout().lock();
    output.write_all(MAGIC)?;
    output.write_all(&identity)?;
    output.write_all(&(stack as u64).to_le_bytes())?;
    output.write_all(&(artifact.len() as u64).to_le_bytes())?;
    output.write_all(&artifact)?;
    output.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires cargo build -p exaudio-wasm --bin exaudio-wasm-worker"]
    fn compiler_process_cancellation_kills_started_child() {
        let limits = Limits::default();
        let engine = Engine::new(&engine_config(limits.wasm_stack_bytes)).unwrap();
        let cancellation = Arc::new(AtomicBool::new(false));
        let token = cancellation.clone();
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            compile_inner(&engine, b"(component)", &limits, &token, |pid| {
                started_tx.send(pid).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            })
        });
        let pid = started_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("compiler child never started");
        assert_ne!(pid, 0);
        cancellation.store(true, Ordering::Relaxed);
        release_tx.send(()).unwrap();
        let error = worker
            .join()
            .unwrap()
            .err()
            .expect("cancelled compile succeeded");
        assert!(format!("{error:#}").contains("cancelled"));
        // compile_inner cannot return until ChildGuard has killed AND reaped it.
    }

    #[test]
    #[ignore = "requires cargo build -p exaudio-wasm --bin exaudio-wasm-worker"]
    fn compiler_process_deadline_and_memory_failure_leave_host_usable() {
        let limits = Limits::default();
        let engine = Engine::new(&engine_config(limits.wasm_stack_bytes)).unwrap();
        let cancellation = Arc::new(AtomicBool::new(false));
        let expired = Limits {
            compile_timeout: Duration::from_nanos(1),
            ..limits.clone()
        };
        let started = Instant::now();
        let error = compile(&engine, b"(component)", &expired, &cancellation)
            .err()
            .expect("expired compile succeeded");
        assert!(format!("{error:#}").contains("deadline"));
        assert!(started.elapsed() < Duration::from_secs(5));
        let exhausted = Limits {
            compiler_memory_bytes: 1,
            ..limits.clone()
        };
        assert!(compile(&engine, b"(component)", &exhausted, &cancellation).is_err());
        assert!(compile(&engine, b"(component)", &limits, &cancellation).is_ok());
    }
}
