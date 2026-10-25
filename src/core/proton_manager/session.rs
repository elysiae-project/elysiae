use super::*;

/// Supported runtime locations that a session fence can reference.
#[derive(Clone, Copy, Serialize, Deserialize)]
enum SessionRoot {
    Canonical,
    LegacyNested,
    LegacyFlat,
}

impl SessionRoot {
    /// Resolves a recorded runtime location under the application data root.
    fn path(self, paths: &RuntimePaths) -> PathBuf {
        match self {
            Self::Canonical => paths.live(),
            Self::LegacyNested => paths.0.join("proton"),
            Self::LegacyFlat => paths.0.clone(),
        }
    }
}

/// Wine architecture pinned when a session starts.
#[derive(Clone, Copy, Serialize, Deserialize)]
enum SessionWine {
    X86_64,
    Arm64,
}

impl SessionWine {
    /// Returns the binary directory and architecture for the recorded Wine variant.
    fn layout(self) -> (&'static str, &'static str) {
        match self {
            Self::X86_64 => ("files/bin", "x86_64"),
            Self::Arm64 => ("files/bin-arm64", "aarch64"),
        }
    }
}

/// Recorded Proton wrapper identity or evidence that it has exited.
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum SessionWrapper {
    #[default]
    Unknown,
    Running {
        pid: u32,
        start_time: u64,
    },
    Reaped,
}

/// Durable session identity that protects the shared prefix until Wine exits.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionFence {
    format: u32,
    root: SessionRoot,
    // Pin the launch-time server variant even if PROTON_USE_ARM64 changes on restart.
    wine: SessionWine,
    boot_id: String,
    #[serde(default)]
    pid_namespace: Option<String>,
    // Older fences cannot prove wrapper exit; treat them as unknown too.
    #[serde(default)]
    wrapper: SessionWrapper,
}

/// Requires a canonical version-4 UUID for a Linux boot identity.
fn validate_boot_id(id: &str) -> Result<()> {
    let uuid = Uuid::parse_str(id).context("Invalid session boot id")?;
    ensure!(
        uuid.to_string() == id
            && uuid.get_version_num() == 4
            && uuid.get_variant() == uuid::Variant::RFC4122,
        "Invalid session boot id"
    );

    Ok(())
}

/// Reads and validates the current Linux boot identity.
fn boot_id() -> Result<String> {
    let bytes = read_metadata(Path::new("/proc/sys/kernel/random/boot_id"), 128)?;
    let id = std::str::from_utf8(&bytes)?.trim().to_owned();
    validate_boot_id(&id)?;

    Ok(id)
}

/// Returns the current process's PID namespace identity.
fn pid_namespace() -> Result<String> {
    Ok(fs::read_link("/proc/self/ns/pid")?
        .to_str()
        .context("Invalid PID namespace identity")?
        .to_owned())
}

/// Rejects mutations whenever a session fence node exists at this root.
fn ensure_no_session_at(paths: &RuntimePaths) -> Result<()> {
    // Any node (including malformed metadata and symlinks) fences mutations.
    ensure!(
        !node_exists(&paths.session())?,
        "A Wine session may still own the shared prefix; session recovery is required"
    );

    Ok(())
}

/// Called by the global operation manager while it owns the profile file lock.
pub(crate) fn ensure_no_session() -> Result<()> {
    ensure_no_session_at(&RuntimePaths::appdata()?)
}

/// Persists a session fence for a known runtime before spawning Proton.
fn fence_session(paths: &RuntimePaths, runtime: &RuntimeLayout) -> Result<SessionFence> {
    ensure_no_session_at(paths)?;
    let root = [
        SessionRoot::Canonical,
        SessionRoot::LegacyNested,
        SessionRoot::LegacyFlat,
    ]
    .into_iter()
    .find(|kind| {
        real_directory(&kind.path(paths), false).is_ok()
            && kind
                .path(paths)
                .canonicalize()
                .is_ok_and(|path| path == runtime.root)
    })
    .context("Session runtime is not a known runtime location")?;
    let wine = if runtime.wineserver == runtime.root.join("files/bin-arm64/wineserver") {
        SessionWine::Arm64
    } else {
        ensure!(
            runtime.wineserver == runtime.root.join("files/bin/wineserver"),
            "Unknown session wineserver"
        );
        SessionWine::X86_64
    };
    let session = SessionFence {
        format: 1,
        root,
        wine,
        boot_id: boot_id()?,
        pid_namespace: Some(pid_namespace()?),
        wrapper: SessionWrapper::Unknown,
    };
    atomic_json(&paths.session(), &session)?;

    Ok(session)
}

/// Persists the wrapper's current identity or exit state in its session fence.
fn record_wrapper(
    paths: &RuntimePaths,
    session: &mut SessionFence,
    wrapper: SessionWrapper,
) -> Result<()> {
    session.wrapper = wrapper;
    atomic_json(&paths.session(), session)
}

/// Process start time and exit state used to recognize a Proton wrapper.
struct WrapperProcess {
    start_time: u64,
    exited: bool,
}

/// Extracts wrapper identity and exit state from Linux process statistics.
fn parse_wrapper_process(pid: u32, stat: &[u8]) -> Result<WrapperProcess> {
    // comm (field 2) may contain spaces, parentheses, or non-UTF8 bytes. Only
    // the final ')' separates it from the ASCII state and numeric fields.
    let end = stat
        .iter()
        .rposition(|byte| *byte == b')')
        .context("Invalid wrapper process stat")?;
    ensure!(
        stat.starts_with(format!("{pid} (").as_bytes()),
        "Wrapper process PID mismatch"
    );

    let mut fields = stat[end + 1..]
        .split(u8::is_ascii_whitespace)
        .filter(|field| !field.is_empty());
    let state = fields.next().context("Missing wrapper process state")?;
    ensure!(state.len() == 1, "Invalid wrapper process state");
    let start_time = std::str::from_utf8(fields.nth(18).context("Missing wrapper start time")?)?
        .parse::<u64>()
        .context("Invalid wrapper start time")?;

    Ok(WrapperProcess {
        start_time,
        exited: matches!(state[0], b'Z' | b'X' | b'x'),
    })
}

/// Inspects a wrapper PID, returning none only when its process entry is absent.
fn wrapper_process(pid: u32) -> Result<Option<WrapperProcess>> {
    ensure!(pid > 0 && pid <= i32::MAX as u32, "Invalid wrapper PID");
    let path = PathBuf::from(format!("/proc/{pid}/stat"));

    match read_metadata(&path, 4096) {
        Ok(stat) => Ok(Some(parse_wrapper_process(pid, &stat)?)),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            // A missing /proc mount is not evidence that the wrapper exited.
            read_metadata(Path::new("/proc/self/stat"), 4096)?;
            Ok(None)
        }
        Err(error) => Err(error).context("Could not inspect Proton wrapper"),
    }
}

/// Checks whether the recorded wrapper exited, accounting for reused PIDs.
fn wrapper_has_exited(pid: u32, start_time: u64) -> Result<bool> {
    Ok(wrapper_process(pid)?
        .is_none_or(|process| process.start_time != start_time || process.exited))
}

/// Waits for a known wrapper to exit without signalling unrelated processes.
fn wait_for_wrapper(wrapper: SessionWrapper) -> Result<()> {
    match wrapper {
        SessionWrapper::Unknown => {
            bail!("Unknown Proton wrapper identity; session remains fenced until reboot")
        }
        SessionWrapper::Reaped => Ok(()),
        SessionWrapper::Running { pid, start_time } => {
            // No signals or waitpid on a possibly unrelated/reused PID. A
            // zombie/dead state also proves it can no longer start Wine.
            while !wrapper_has_exited(pid, start_time)? {
                std::thread::sleep(Duration::from_millis(50));
            }

            Ok(())
        }
    }
}

/// Removes a session fence and syncs its parent directory.
fn clear_session(paths: &RuntimePaths) -> Result<()> {
    fs::remove_file(paths.session())?;
    sync_directory(&paths.0)
}

/// Waits for Wine in the recorded prefix before clearing the session fence.
fn quiesce_session(paths: &RuntimePaths, runtime: &RuntimeLayout) -> Result<()> {
    // Unlike an update of a never-used runtime, a recorded session must have a
    // real prefix. A missing/replaced prefix cannot establish quiescence.
    real_directory(&paths.prefix(), false)?;
    if node_exists(&paths.prefix().join("pfx"))? {
        real_directory(&paths.prefix().join("pfx"), false)?;
    }

    wait_for_session(paths, runtime)?;
    clear_session(paths)
}

/// Recovers a fenced session after a reboot or confirmed wrapper and Wine exit.
fn recover_session_at(paths: &RuntimePaths) -> Result<()> {
    if !node_exists(&paths.session())? {
        return Ok(());
    }

    let mut session: SessionFence = read_json(&paths.session())?;
    ensure!(session.format == 1, "Unsupported session fence format");
    validate_boot_id(&session.boot_id)?;
    if session.boot_id != boot_id()? {
        // Linux processes cannot survive a reboot; no runtime execution needed.
        return clear_session(paths);
    }
    ensure!(
        session.pid_namespace.as_deref() == Some(pid_namespace()?.as_str()),
        "Session PID namespace is missing or different; session remains fenced until reboot"
    );
    let (bin, architecture) = session.wine.layout();
    let runtime =
        RuntimeLayout::validate_architecture(&session.root.path(paths), bin, architecture)?;
    wait_for_wrapper(session.wrapper)?;
    record_wrapper(paths, &mut session, SessionWrapper::Reaped)?;
    quiesce_session(paths, &runtime)
}

/// Recovery is the sole operation allowed past a durable session fence. The
/// blocking worker retains its lease/profile lock even if the caller is dropped.
pub async fn recover_session() -> Result<()> {
    recover_session_with(RuntimePaths::appdata()?, operations()).await
}

/// Recovers an existing session while the supplied operation manager holds its lease.
async fn recover_session_with(paths: RuntimePaths, owner: &Operations) -> Result<()> {
    if !node_exists(&paths.session())? {
        return Ok(());
    }

    let operation = owner.begin(OperationKind::Recovery, None)?;
    runtime_worker(operation, move |_| recover_session_at(&paths)).await
}

/// Configures a child's shared-prefix environment without changing the host process.
fn prefix_environment(command: &mut Command, paths: &RuntimePaths) {
    command
        .env("STEAM_COMPAT_DATA_PATH", paths.prefix())
        .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", "")
        .env("WINEPREFIX", paths.prefix().join("pfx"));
}

/// Builds a Proton launch command for an absolute game path and the shared prefix.
fn game_command(
    paths: &RuntimePaths,
    runtime: &RuntimeLayout,
    executable: &Path,
) -> Result<Command> {
    ensure!(
        executable.is_absolute(),
        "Game executable must be an absolute path"
    );
    let executable = executable
        .canonicalize()
        .context("Game executable could not be resolved")?;
    ensure!(executable.is_file(), "Game executable is not a file");
    let root = executable
        .parent()
        .context("Game executable has no parent")?;
    real_directory(&paths.prefix(), true)?;
    if node_exists(&paths.prefix().join("pfx"))? {
        real_directory(&paths.prefix().join("pfx"), false)?;
    }

    let mut command = Command::new(&runtime.executable);
    command.arg("run").arg(&executable).current_dir(root);
    prefix_environment(&mut command, paths);

    Ok(command)
}

/// Reaps an owned child, attempting termination if waiting fails.
fn wait_child(mut child: Child) -> Result<ExitStatus> {
    match child.wait() {
        Ok(status) => Ok(status),
        Err(error) => {
            // Do not leave a live child behind on an unexpected wait failure.
            let _ = child.kill();
            let _ = child.wait();

            Err(error).context("Could not reap runtime process")
        }
    }
}

/// Waits for Wine to exit using the runtime's matching server and libraries.
pub(super) fn wait_for_session(paths: &RuntimePaths, runtime: &RuntimeLayout) -> Result<()> {
    if !node_exists(&paths.prefix())? {
        return Ok(());
    }

    real_directory(&paths.prefix(), false)?;
    let mut command = Command::new(&runtime.wineserver);
    command.arg("-w").current_dir(&runtime.root);
    prefix_environment(&mut command, paths);

    // Phlogiston 11 uses multiarch library directories; older GE releases use
    // lib/lib64. Match Proton's search order without changing the host process.
    let mut libraries = Vec::new();
    if runtime.wineserver == runtime.root.join("files/bin-arm64/wineserver") {
        libraries.push(runtime.root.join("files/lib/aarch64-linux-gnu"));
    }

    libraries.extend([
        runtime.root.join("files/lib/x86_64-linux-gnu"),
        runtime.root.join("files/lib/i386-linux-gnu"),
        runtime.root.join("files/lib"),
        runtime.root.join("files/lib64"),
    ]);
    if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH") {
        libraries.extend(std::env::split_paths(&existing));
    }

    command.env("LD_LIBRARY_PATH", std::env::join_paths(libraries)?);
    let status = wait_child(
        command
            .spawn()
            .context("Could not start matching wineserver -w")?,
    )?;
    ensure!(status.success(), "wineserver -w failed: {status}");

    Ok(())
}

/// Launches and supervises a game through Proton under the supplied operation lease.
pub async fn run_game(executable: PathBuf, operation: Operation) -> Result<()> {
    run_game_at(RuntimePaths::appdata()?, executable, operation).await
}

/// Runs game supervision at an explicit root in a lease-owning worker.
async fn run_game_at(paths: RuntimePaths, executable: PathBuf, operation: Operation) -> Result<()> {
    runtime_worker(operation, move |operation| {
        run_game_owned(&paths, &executable, operation)
    })
    .await
}

/// Fences and supervises Proton and Wine, retaining the fence if exit is uncertain.
fn run_game_owned(paths: &RuntimePaths, executable: &Path, operation: &Operation) -> Result<()> {
    operation.check_cancelled()?;
    ensure_no_session_at(paths)?;
    let runtime = paths.resolve()?;
    let mut command = game_command(paths, &runtime, executable)?;

    // Persist before spawn: a crash or any supervision uncertainty must
    // continue excluding other processes after this lease is released.
    let mut fence = fence_session(paths, &runtime)?;
    let mut wrapper_reaped = true;
    let launched = match command.spawn().context("Could not start Proton") {
        Err(error) => Err(error),
        Ok(child) => {
            let pid = child.id();
            // Do not return early on /proc or publication failure: this worker
            // still owns the child and must reap it before releasing its lease.
            let recorded = (|| {
                let process =
                    wrapper_process(pid)?.context("Proton wrapper identity disappeared")?;
                record_wrapper(
                    paths,
                    &mut fence,
                    SessionWrapper::Running {
                        pid,
                        start_time: process.start_time,
                    },
                )
            })();
            let waited = wait_child(child);
            wrapper_reaped = waited.is_ok();

            match (recorded, waited) {
                (Err(error), Err(wait)) => {
                    Err(error).context(format!("Wrapper wait also failed: {wait:#}"))
                }
                (Err(error), _) => Err(error),
                (_, Err(error)) => Err(error),
                (Ok(()), Ok(status)) => {
                    if status.success() {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("Proton exited unsuccessfully: {status}"))
                    }
                }
            }
        }
    };

    if !wrapper_reaped {
        // Even successful -w is premature while this wrapper might start Wine.
        return launched.context("Wrapper exit uncertain; session fence retained");
    }

    // Publish known exit (also after spawn failure) before clearing the fence.
    // Publication errors retain Unknown/Running/Reaped, all safe to recover.
    let session = record_wrapper(paths, &mut fence, SessionWrapper::Reaped)
        .and_then(|_| quiesce_session(paths, &runtime));

    match (launched, session) {
        (Err(error), Err(wait)) => {
            Err(error).context(format!("Wine session wait also failed: {wait:#}"))
        }
        (Err(error), _) => Err(error),
        (_, result) => result,
    }
}
