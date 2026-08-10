use std::borrow::Cow;
use std::collections::VecDeque;
use std::fmt::{Display, Formatter};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub mod catalog;

/// Run a protocol engine directly on the caller's standard streams while
/// retaining the same kill-on-close and memory protections as managed sessions.
pub fn run_passthrough_with_memory_limit(
    path: &Path,
    args: &[String],
    memory_limit_bytes: Option<u64>,
) -> Result<std::process::ExitStatus, String> {
    run_passthrough_with_environment(path, args, &[], memory_limit_bytes)
}

/// Run a contained passthrough engine with additional inherited environment
/// entries while leaving its command line and protocol stream untouched.
pub fn run_passthrough_with_environment(
    path: &Path,
    args: &[String],
    environment: &[(&str, &str)],
    memory_limit_bytes: Option<u64>,
) -> Result<std::process::ExitStatus, String> {
    let mut command = Command::new(path);
    command
        .args(args)
        .envs(environment.iter().copied())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    configure_no_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to start '{}': {error}", path.display()))?;
    let _job = match process_safety::KillOnCloseJob::attach(&child, memory_limit_bytes) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("failed to constrain '{}': {error}", path.display()));
        }
    };
    child
        .wait()
        .map_err(|error| format!("failed to wait for '{}': {error}", path.display()))
}

/// A root-level UCI adapter for a complete native search/evaluation stack.
///
/// Implementations may adjust protocol metadata, but search commands and
/// telemetry remain owned by the native engine. This keeps network-specific
/// search assumptions together without adding work to either engine's hot path.
pub trait UciSearchStackAdapter {
    /// Transform controller input before it reaches the native search stack.
    /// This runs only on UCI command boundaries, never in the search hot path.
    fn transform_controller_line<'a>(&self, line: &'a str) -> Cow<'a, str> {
        Cow::Borrowed(line)
    }

    fn transform_engine_line<'a>(&self, line: &'a str) -> Cow<'a, str>;
}

/// Rebrands UCI identity fields while preserving options, analysis, NPS and
/// move selection exactly as emitted by the native backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UciIdentityAdapter<'a> {
    pub name: &'a str,
    pub author: &'a str,
}

impl UciSearchStackAdapter for UciIdentityAdapter<'_> {
    fn transform_engine_line<'a>(&self, line: &'a str) -> Cow<'a, str> {
        let (content, terminator) = if let Some(content) = line.strip_suffix("\r\n") {
            (content, "\r\n")
        } else if let Some(content) = line.strip_suffix('\n') {
            (content, "\n")
        } else {
            (line, "")
        };

        if content.starts_with("id name ") {
            Cow::Owned(format!("id name {}{terminator}", self.name))
        } else if content.starts_with("id author ") {
            Cow::Owned(format!("id author {}{terminator}", self.author))
        } else {
            Cow::Borrowed(line)
        }
    }
}

/// Identity and hard resource boundary for a complete native search stack.
///
/// The advertised and accepted `Hash`/`Threads` ranges are kept inside the
/// process job's budget. Search telemetry and `bestmove` remain byte-for-byte
/// native output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundedUciIdentityAdapter<'a> {
    pub identity: UciIdentityAdapter<'a>,
    pub max_hash_mb: usize,
    pub max_threads: usize,
}

impl UciSearchStackAdapter for BoundedUciIdentityAdapter<'static> {
    fn transform_controller_line<'a>(&self, line: &'a str) -> Cow<'a, str> {
        clamp_setoption(line, "Hash", self.max_hash_mb)
            .or_else(|| clamp_setoption(line, "Threads", self.max_threads))
            .unwrap_or(Cow::Borrowed(line))
    }

    fn transform_engine_line<'a>(&self, line: &'a str) -> Cow<'a, str> {
        let identity = self.identity.transform_engine_line(line);
        if matches!(identity, Cow::Owned(_)) {
            return identity;
        }
        clamp_option_max(line, "Hash", self.max_hash_mb)
            .or_else(|| clamp_option_max(line, "Threads", self.max_threads))
            .unwrap_or(identity)
    }
}

fn line_parts(line: &str) -> (&str, &str) {
    if let Some(content) = line.strip_suffix("\r\n") {
        (content, "\r\n")
    } else if let Some(content) = line.strip_suffix('\n') {
        (content, "\n")
    } else {
        (line, "")
    }
}

fn clamp_setoption<'a>(line: &'a str, option: &str, maximum: usize) -> Option<Cow<'a, str>> {
    let (content, terminator) = line_parts(line);
    let tokens = content.split_whitespace().collect::<Vec<_>>();
    if tokens.len() != 5
        || !tokens[0].eq_ignore_ascii_case("setoption")
        || !tokens[1].eq_ignore_ascii_case("name")
        || !tokens[2].eq_ignore_ascii_case(option)
        || !tokens[3].eq_ignore_ascii_case("value")
    {
        return None;
    }
    let requested = tokens[4].parse::<usize>().ok()?;
    (requested > maximum).then(|| {
        Cow::Owned(format!(
            "setoption name {option} value {maximum}{terminator}"
        ))
    })
}

fn clamp_option_max<'a>(line: &'a str, option: &str, maximum: usize) -> Option<Cow<'a, str>> {
    let (content, terminator) = line_parts(line);
    let mut tokens = content
        .split_whitespace()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if tokens.len() < 10
        || !tokens[0].eq_ignore_ascii_case("option")
        || !tokens[1].eq_ignore_ascii_case("name")
        || !tokens[2].eq_ignore_ascii_case(option)
    {
        return None;
    }
    let max_index = tokens
        .iter()
        .position(|token| token.eq_ignore_ascii_case("max"))?
        .checked_add(1)?;
    let advertised = tokens.get(max_index)?.parse::<usize>().ok()?;
    if advertised <= maximum {
        return None;
    }
    tokens[max_index] = maximum.to_string();
    Some(Cow::Owned(format!("{}{terminator}", tokens.join(" "))))
}

fn relay_uci_output<R: BufRead, W: Write, A: UciSearchStackAdapter>(
    mut reader: R,
    mut writer: W,
    adapter: &A,
) -> std::io::Result<()> {
    let mut line = String::with_capacity(256);
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        writer.write_all(adapter.transform_engine_line(&line).as_bytes())?;
        writer.flush()?;
    }
    Ok(())
}

fn relay_uci_input<R: BufRead, W: Write, A: UciSearchStackAdapter>(
    mut reader: R,
    mut writer: W,
    adapter: &A,
) -> std::io::Result<()> {
    let mut line = String::with_capacity(128);
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        writer.write_all(adapter.transform_controller_line(&line).as_bytes())?;
        writer.flush()?;
    }
    Ok(())
}

/// Run a complete native UCI backend behind a metadata-only adapter.
///
/// Input is copied without parsing. Output is streamed line by line so GUIs
/// receive opening analysis immediately. A backend EOF or crash terminates the
/// adapter instead of leaving a disconnected process alive.
pub fn run_uci_search_stack_adapter<A>(
    path: &Path,
    args: &[String],
    environment: &[(&str, &str)],
    memory_limit_bytes: Option<u64>,
    adapter: &A,
) -> Result<std::process::ExitStatus, String>
where
    A: UciSearchStackAdapter + Clone + Send + 'static,
{
    let mut command = Command::new(path);
    command
        .args(args)
        .envs(environment.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    configure_no_window(&mut command);

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to start '{}': {error}", path.display()))?;
    let _job = match process_safety::KillOnCloseJob::attach(&child, memory_limit_bytes) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("failed to constrain '{}': {error}", path.display()));
        }
    };
    let mut child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| format!("failed to open stdin for '{}'", path.display()))?;
    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("failed to open stdout for '{}'", path.display()))?;

    let input_adapter = adapter.clone();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let _ = relay_uci_input(stdin.lock(), &mut child_stdin, &input_adapter);
    });

    if let Err(error) = relay_uci_output(
        BufReader::new(child_stdout),
        std::io::stdout().lock(),
        adapter,
    ) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!(
            "failed to relay output from '{}': {error}",
            path.display()
        ));
    }

    let deadline = Instant::now() + SHUTDOWN_GRACE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "'{}' disconnected from UCI output without exiting",
                    path.display()
                ));
            }
            Err(error) => {
                return Err(format!("failed to wait for '{}': {error}", path.display()));
            }
        }
    }
}

#[cfg(windows)]
fn configure_no_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_no_window(_command: &mut Command) {}

const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(20);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(250);
const RESOURCE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const STDERR_TAIL_LINES: usize = 16;
const STDERR_LINE_BYTES: usize = 512;

#[cfg(windows)]
mod process_safety {
    use std::ffi::c_void;
    use std::io;
    use std::mem::{size_of, zeroed};
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::ptr;

    type Handle = *mut c_void;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
    const JOB_OBJECT_LIMIT_JOB_MEMORY: u32 = 0x0000_0200;

    #[repr(C)]
    struct JobObjectBasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    struct JobObjectExtendedLimitInformation {
        basic_limit_information: JobObjectBasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(
            job: Handle,
            information_class: i32,
            information: *const c_void,
            information_length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn CloseHandle(object: Handle) -> i32;
        fn K32GetProcessMemoryInfo(
            process: Handle,
            counters: *mut ProcessMemoryCounters,
            size: u32,
        ) -> i32;
    }

    pub struct KillOnCloseJob(usize);

    impl KillOnCloseJob {
        pub fn attach(child: &Child, memory_limit_bytes: Option<u64>) -> io::Result<Self> {
            // SAFETY: Windows owns the returned handle; all structures match the
            // documented ABI and the handle is closed by `Drop`.
            unsafe {
                let job = CreateJobObjectW(ptr::null(), ptr::null());
                if job.is_null() {
                    return Err(io::Error::last_os_error());
                }

                let mut limits: JobObjectExtendedLimitInformation = zeroed();
                limits.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if let Some(limit) = memory_limit_bytes {
                    limits.basic_limit_information.limit_flags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
                    limits.job_memory_limit = limit.min(usize::MAX as u64) as usize;
                }
                if SetInformationJobObject(
                    job,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                    (&raw const limits).cast(),
                    size_of::<JobObjectExtendedLimitInformation>() as u32,
                ) == 0
                    || AssignProcessToJobObject(job, child.as_raw_handle().cast()) == 0
                {
                    let error = io::Error::last_os_error();
                    CloseHandle(job);
                    return Err(error);
                }
                Ok(Self(job as usize))
            }
        }
    }

    impl Drop for KillOnCloseJob {
        fn drop(&mut self) {
            // SAFETY: this object exclusively owns the job handle.
            unsafe {
                CloseHandle(self.0 as Handle);
            }
        }
    }

    pub fn working_set_bytes(child: &Child) -> io::Result<u64> {
        // SAFETY: `child` owns a live process handle and the output structure
        // matches the documented PROCESS_MEMORY_COUNTERS layout.
        unsafe {
            let mut counters: ProcessMemoryCounters = zeroed();
            counters.cb = size_of::<ProcessMemoryCounters>() as u32;
            if K32GetProcessMemoryInfo(child.as_raw_handle().cast(), &raw mut counters, counters.cb)
                == 0
            {
                Err(io::Error::last_os_error())
            } else {
                Ok(counters.working_set_size as u64)
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        use std::thread;
        use std::time::{Duration, Instant};

        #[test]
        fn closing_job_terminates_assigned_process() {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let mut child = Command::new("cmd.exe")
                .args(["/C", "ping -n 30 127.0.0.1 >NUL"])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .expect("spawn job test child");
            let job = KillOnCloseJob::attach(&child, None).expect("attach job");

            drop(job);

            let deadline = Instant::now() + Duration::from_secs(2);
            while child.try_wait().expect("query child").is_none() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            assert!(child.try_wait().expect("query child").is_some());
        }
    }
}

#[cfg(not(windows))]
mod process_safety {
    use std::io;
    use std::process::Child;

    pub struct KillOnCloseJob;

    impl KillOnCloseJob {
        pub fn attach(_child: &Child, _memory_limit_bytes: Option<u64>) -> io::Result<Self> {
            Ok(Self)
        }
    }

    pub fn working_set_bytes(_child: &Child) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "working-set monitoring is unavailable on this platform",
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolKind {
    Uci,
    Xboard,
}

impl Display for ProtocolKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Uci => write!(f, "uci"),
            Self::Xboard => write!(f, "xboard"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct EngineOptions {
    pub hash_mb: Option<usize>,
    pub threads: Option<usize>,
    pub own_book: Option<bool>,
    /// Additional UCI options, applied after the common options above.
    pub custom: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct SearchRequest {
    /// Initial FEN for the position history.
    pub fen: String,
    /// UCI moves played from `fen`, preserving repetition history for matches.
    pub moves: Vec<String>,
    pub depth: i32,
    pub movetime: Option<Duration>,
    pub node_limit: Option<u64>,
}

impl SearchRequest {
    pub fn depth_only(fen: String, depth: i32) -> Self {
        Self {
            fen,
            moves: Vec::new(),
            depth,
            movetime: None,
            node_limit: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SearchInfo {
    pub best_move: String,
    pub ponder_move: Option<String>,
    pub depth: i32,
    pub seldepth: i32,
    pub score: i32,
    pub nodes: u64,
    pub nps: u64,
    pub time_ms: u64,
    pub hashfull: u16,
    pub tablebase_hits: u64,
    pub current_move: Option<String>,
    pub current_move_number: u32,
    pub pv: Vec<String>,
    pub message: Option<String>,
}

pub trait ProtocolDriver {
    fn initialize(&mut self, io: &mut EngineIo) -> Result<(), String>;
    fn configure(&mut self, io: &mut EngineIo, options: &EngineOptions) -> Result<(), String>;
    fn new_game(&mut self, io: &mut EngineIo) -> Result<(), String>;
    fn set_position(
        &mut self,
        io: &mut EngineIo,
        fen: &str,
        moves: &[String],
    ) -> Result<(), String>;
    fn start_search(&mut self, io: &mut EngineIo, req: &SearchRequest) -> Result<(), String>;
    fn start_ponder(&mut self, _io: &mut EngineIo, _req: &SearchRequest) -> Result<(), String> {
        Err("pondering is not supported by this protocol".to_owned())
    }
    fn ponder_hit(&mut self, _io: &mut EngineIo) -> Result<(), String> {
        Err("ponderhit is not supported by this protocol".to_owned())
    }
    fn stop_search(&mut self, _io: &mut EngineIo) -> Result<(), String> {
        Err("stopping an active search is not supported by this protocol".to_owned())
    }
    fn parse_output_line(&mut self, line: &str, info: &mut SearchInfo) -> Option<String>;

    fn quit(&mut self, io: &mut EngineIo) -> Result<(), String> {
        io.send("quit")
    }
}

pub struct EngineIo {
    child: Child,
    _kill_on_close: process_safety::KillOnCloseJob,
    stdin: ChildStdin,
    stdout_rx: Receiver<String>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    read_timeout: Duration,
    memory_limit_bytes: Option<u64>,
}

impl EngineIo {
    fn spawn_bounded(
        path: &Path,
        args: &[String],
        memory_limit_bytes: Option<u64>,
    ) -> Result<Self, String> {
        let mut command = Command::new(path);
        command.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to spawn engine '{}': {e}", path.display()))?;

        let kill_on_close = match process_safety::KillOnCloseJob::attach(&child, memory_limit_bytes)
        {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("failed to contain engine process: {error}"));
            }
        };
        let stdin = child.stdin.take().ok_or("failed to open engine stdin")?;
        let stdout = child.stdout.take().ok_or("failed to open engine stdout")?;
        let stderr = child.stderr.take().ok_or("failed to open engine stderr")?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || pump_stdout(stdout, tx));
        let stderr_tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)));
        let stderr_sink = Arc::clone(&stderr_tail);
        std::thread::spawn(move || pump_stderr(stderr, &stderr_sink));

        Ok(Self {
            child,
            _kill_on_close: kill_on_close,
            stdin,
            stdout_rx: rx,
            stderr_tail,
            read_timeout: DEFAULT_READ_TIMEOUT,
            memory_limit_bytes,
        })
    }

    pub fn send(&mut self, cmd: &str) -> Result<(), String> {
        writeln!(self.stdin, "{cmd}").map_err(|e| format!("write error: {e}"))?;
        self.stdin
            .flush()
            .map_err(|e| format!("flush error: {e}"))?;
        Ok(())
    }

    pub fn read_line(&mut self) -> Result<String, String> {
        let deadline = Instant::now() + self.read_timeout;
        loop {
            self.enforce_memory_limit()?;
            let now = Instant::now();
            if now >= deadline {
                return Err(format!(
                    "read timeout after {} ms",
                    self.read_timeout.as_millis()
                ));
            }
            let wait = RESOURCE_POLL_INTERVAL.min(deadline.saturating_duration_since(now));
            match self.stdout_rx.recv_timeout(wait) {
                Ok(line) => {
                    self.enforce_memory_limit()?;
                    return Ok(line);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let status =
                        self.child.try_wait().ok().flatten().map_or_else(
                            || "unknown status".to_string(),
                            |status| status.to_string(),
                        );
                    let tail = self
                        .stderr_tail
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" | ");
                    return if tail.is_empty() {
                        Err(format!("engine closed stdout ({status})"))
                    } else {
                        Err(format!("engine closed stdout ({status}): {tail}"))
                    };
                }
            }
        }
    }

    pub fn try_read_line(&mut self) -> Result<Option<String>, String> {
        self.enforce_memory_limit()?;
        match self.stdout_rx.try_recv() {
            Ok(line) => {
                self.enforce_memory_limit()?;
                Ok(Some(line))
            }
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                let status = self
                    .child
                    .try_wait()
                    .ok()
                    .flatten()
                    .map_or_else(|| "unknown status".to_owned(), |status| status.to_string());
                let tail = self
                    .stderr_tail
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" | ");
                if tail.is_empty() {
                    Err(format!("engine closed stdout ({status})"))
                } else {
                    Err(format!("engine closed stdout ({status}): {tail}"))
                }
            }
        }
    }

    pub fn set_read_timeout(&mut self, timeout: Duration) {
        self.read_timeout = timeout;
    }

    fn set_memory_limit_bytes(&mut self, limit: Option<u64>) {
        self.memory_limit_bytes = limit;
    }

    fn enforce_memory_limit(&mut self) -> Result<(), String> {
        let Some(limit) = self.memory_limit_bytes else {
            return Ok(());
        };
        let Ok(working_set) = process_safety::working_set_bytes(&self.child) else {
            return Ok(());
        };
        if working_set <= limit {
            return Ok(());
        }

        let _ = self.child.kill();
        let _ = self.child.wait();
        Err(format!(
            "engine working set exceeded limit: {} MiB > {} MiB",
            working_set / (1024 * 1024),
            limit / (1024 * 1024),
        ))
    }
}

fn pump_stdout(stdout: ChildStdout, tx: mpsc::Sender<String>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if tx.send(line.trim_end().to_string()).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

fn pump_stderr(stderr: ChildStderr, tail: &Mutex<VecDeque<String>>) {
    let reader = BufReader::new(stderr);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        record_stderr_line(tail, &line);
    }
}

fn record_stderr_line(tail: &Mutex<VecDeque<String>>, line: &str) {
    let mut line = line.trim_end().to_string();
    line.truncate(line.floor_char_boundary(STDERR_LINE_BYTES));
    let mut tail = tail.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if tail.len() == STDERR_TAIL_LINES {
        tail.pop_front();
    }
    tail.push_back(line);
}

impl Drop for EngineIo {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "quit");
        let _ = self.stdin.flush();
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                _ => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineSearchState {
    Idle,
    Searching,
    Pondering,
}

pub struct EngineSession {
    io: EngineIo,
    driver: Box<dyn ProtocolDriver + Send>,
    search_state: EngineSearchState,
    pending_info: SearchInfo,
}

impl EngineSession {
    pub fn spawn(path: &Path, protocol: ProtocolKind) -> Result<Self, String> {
        Self::spawn_with_args(path, &[], protocol)
    }

    pub fn spawn_with_args(
        path: &Path,
        args: &[String],
        protocol: ProtocolKind,
    ) -> Result<Self, String> {
        Self::spawn_with_args_and_memory_limit(path, args, protocol, None)
    }

    pub fn spawn_with_args_and_memory_limit(
        path: &Path,
        args: &[String],
        protocol: ProtocolKind,
        memory_limit_bytes: Option<u64>,
    ) -> Result<Self, String> {
        let mut io = EngineIo::spawn_bounded(path, args, memory_limit_bytes)?;
        let mut driver: Box<dyn ProtocolDriver + Send> = match protocol {
            ProtocolKind::Uci => Box::new(UciDriver),
            ProtocolKind::Xboard => Box::new(XboardDriver),
        };
        driver.initialize(&mut io)?;
        Ok(Self {
            io,
            driver,
            search_state: EngineSearchState::Idle,
            pending_info: SearchInfo::default(),
        })
    }

    pub fn set_read_timeout(&mut self, timeout: Duration) {
        self.io.set_read_timeout(timeout);
    }

    pub fn set_memory_limit_bytes(&mut self, limit: Option<u64>) {
        self.io.set_memory_limit_bytes(limit);
    }

    pub fn process_id(&self) -> u32 {
        self.io.child.id()
    }

    pub fn configure(&mut self, options: &EngineOptions) -> Result<(), String> {
        self.require_idle("configure the engine")?;
        self.driver.configure(&mut self.io, options)
    }

    pub fn new_game(&mut self) -> Result<(), String> {
        self.require_idle("start a new game")?;
        self.driver.new_game(&mut self.io)
    }

    pub const fn search_state(&self) -> EngineSearchState {
        self.search_state
    }

    pub fn start_search(&mut self, req: &SearchRequest) -> Result<(), String> {
        self.begin_search(req, false)
    }

    pub fn start_ponder(&mut self, req: &SearchRequest) -> Result<(), String> {
        self.begin_search(req, true)
    }

    fn begin_search(&mut self, req: &SearchRequest, ponder: bool) -> Result<(), String> {
        self.require_idle(if ponder {
            "start pondering"
        } else {
            "start a search"
        })?;
        self.driver
            .set_position(&mut self.io, &req.fen, &req.moves)?;
        if ponder {
            self.driver.start_ponder(&mut self.io, req)?;
            self.search_state = EngineSearchState::Pondering;
        } else {
            self.driver.start_search(&mut self.io, req)?;
            self.search_state = EngineSearchState::Searching;
        }
        self.pending_info = SearchInfo::default();
        Ok(())
    }

    pub fn ponder_hit(&mut self) -> Result<(), String> {
        if self.search_state != EngineSearchState::Pondering {
            return Err("cannot send ponderhit without an active ponder search".to_owned());
        }
        self.driver.ponder_hit(&mut self.io)?;
        self.search_state = EngineSearchState::Searching;
        Ok(())
    }

    pub fn poll_search(&mut self) -> Result<Option<SearchInfo>, String> {
        self.require_active("poll a search")?;
        loop {
            let line = match self.io.try_read_line() {
                Ok(Some(line)) => line,
                Ok(None) => return Ok(None),
                Err(error) => {
                    self.reset_search_state();
                    return Err(error);
                }
            };
            if let Some(result) = self.consume_search_line(&line) {
                return Ok(Some(result));
            }
        }
    }

    pub fn wait_for_bestmove(&mut self) -> Result<SearchInfo, String> {
        self.require_active("wait for bestmove")?;
        loop {
            let line = match self.io.read_line() {
                Ok(line) => line,
                Err(error) => {
                    self.reset_search_state();
                    return Err(error);
                }
            };
            if let Some(result) = self.consume_search_line(&line) {
                return Ok(result);
            }
        }
    }

    pub fn stop_search(&mut self) -> Result<SearchInfo, String> {
        self.require_active("stop a search")?;
        if let Err(error) = self.driver.stop_search(&mut self.io) {
            self.reset_search_state();
            return Err(error);
        }
        self.wait_for_bestmove()
    }

    pub fn search(&mut self, req: &SearchRequest) -> Result<SearchInfo, String> {
        self.start_search(req)?;
        self.wait_for_bestmove()
    }

    fn consume_search_line(&mut self, line: &str) -> Option<SearchInfo> {
        let best = self
            .driver
            .parse_output_line(line, &mut self.pending_info)?;
        self.pending_info.best_move = best;
        self.search_state = EngineSearchState::Idle;
        Some(std::mem::take(&mut self.pending_info))
    }

    fn require_idle(&self, operation: &str) -> Result<(), String> {
        if self.search_state == EngineSearchState::Idle {
            Ok(())
        } else {
            Err(format!(
                "cannot {operation} while engine is {:?}",
                self.search_state
            ))
        }
    }

    fn require_active(&self, operation: &str) -> Result<(), String> {
        if self.search_state == EngineSearchState::Idle {
            Err(format!("cannot {operation} while engine is idle"))
        } else {
            Ok(())
        }
    }

    fn reset_search_state(&mut self) {
        self.search_state = EngineSearchState::Idle;
        self.pending_info = SearchInfo::default();
    }
}

pub fn analyze_once(
    path: &Path,
    protocol: ProtocolKind,
    options: &EngineOptions,
    req: &SearchRequest,
) -> Result<SearchInfo, String> {
    let mut session = EngineSession::spawn(path, protocol)?;
    session.configure(options)?;
    session.search(req)
}

#[derive(Default)]
struct UciDriver;

impl UciDriver {
    fn parse_info_line(&self, line: &str, info: &mut SearchInfo) {
        let mut tokens = line.split_whitespace();
        if tokens.next() != Some("info") {
            return;
        }
        while let Some(token) = tokens.next() {
            match token {
                "depth" => {
                    if let Some(v) = tokens.next().and_then(|value| value.parse().ok()) {
                        info.depth = v;
                    }
                }
                "seldepth" => {
                    if let Some(v) = tokens.next().and_then(|value| value.parse().ok()) {
                        info.seldepth = v;
                    }
                }
                "score" => {
                    let kind = tokens.next();
                    let value = tokens.next().and_then(|value| value.parse::<i32>().ok());
                    match (kind, value) {
                        (Some("cp"), Some(score)) => info.score = score,
                        (Some("mate"), Some(mate)) => {
                            info.score = if mate > 0 {
                                29_000 - mate
                            } else {
                                -29_000 - mate
                            };
                        }
                        _ => {}
                    }
                }
                "nodes" => {
                    if let Some(v) = tokens.next().and_then(|value| value.parse().ok()) {
                        info.nodes = v;
                    }
                }
                "nps" => {
                    if let Some(v) = tokens.next().and_then(|value| value.parse().ok()) {
                        info.nps = v;
                    }
                }
                "time" => {
                    if let Some(v) = tokens.next().and_then(|value| value.parse().ok()) {
                        info.time_ms = v;
                    }
                }
                "hashfull" => {
                    if let Some(v) = tokens.next().and_then(|value| value.parse().ok()) {
                        info.hashfull = v;
                    }
                }
                "tbhits" => {
                    if let Some(v) = tokens.next().and_then(|value| value.parse().ok()) {
                        info.tablebase_hits = v;
                    }
                }
                "currmove" => info.current_move = tokens.next().map(ToOwned::to_owned),
                "currmovenumber" => {
                    if let Some(v) = tokens.next().and_then(|value| value.parse().ok()) {
                        info.current_move_number = v;
                    }
                }
                "pv" => {
                    info.pv.clear();
                    info.pv.extend(tokens.map(ToOwned::to_owned));
                    break;
                }
                "string" => {
                    info.message = Some(tokens.collect::<Vec<_>>().join(" "));
                    break;
                }
                _ => {}
            }
        }
    }
}

fn uci_setoption_command(name: &str, value: &str) -> Result<String, String> {
    let name = name.trim();
    let value = value.trim();
    if name.is_empty() {
        return Err("UCI option name cannot be empty".to_string());
    }
    if name.chars().any(char::is_control) || value.chars().any(char::is_control) {
        return Err("UCI option names and values cannot contain control characters".to_string());
    }
    Ok(format!("setoption name {name} value {value}"))
}

fn uci_position_command(fen: &str, moves: &[String]) -> String {
    let mut command = format!("position fen {fen}");
    if !moves.is_empty() {
        command.push_str(" moves ");
        command.push_str(&moves.join(" "));
    }
    command
}

fn uci_initialization_commands() -> [&'static str; 2] {
    // A leading no-op makes startup tolerant of clients that emit a UTF-8 BOM
    // before their first redirected command.
    ["", "uci"]
}

impl ProtocolDriver for UciDriver {
    fn initialize(&mut self, io: &mut EngineIo) -> Result<(), String> {
        for command in uci_initialization_commands() {
            io.send(command)?;
        }
        loop {
            let line = io.read_line()?;
            if line == "uciok" {
                break;
            }
        }
        io.send("isready")?;
        loop {
            if io.read_line()? == "readyok" {
                break;
            }
        }
        Ok(())
    }

    fn configure(&mut self, io: &mut EngineIo, options: &EngineOptions) -> Result<(), String> {
        if let Some(hash_mb) = options.hash_mb {
            io.send(&format!("setoption name Hash value {hash_mb}"))?;
        }
        if let Some(threads) = options.threads {
            io.send(&format!("setoption name Threads value {threads}"))?;
        }
        if let Some(own_book) = options.own_book {
            io.send(&format!("setoption name OwnBook value {own_book}"))?;
        }
        for (name, value) in &options.custom {
            io.send(&uci_setoption_command(name, value)?)?;
        }
        io.send("isready")?;
        loop {
            if io.read_line()? == "readyok" {
                break;
            }
        }
        Ok(())
    }

    fn new_game(&mut self, io: &mut EngineIo) -> Result<(), String> {
        io.send("ucinewgame")?;
        io.send("isready")?;
        loop {
            if io.read_line()? == "readyok" {
                return Ok(());
            }
        }
    }

    fn set_position(
        &mut self,
        io: &mut EngineIo,
        fen: &str,
        moves: &[String],
    ) -> Result<(), String> {
        io.send(&uci_position_command(fen, moves))
    }

    fn start_search(&mut self, io: &mut EngineIo, req: &SearchRequest) -> Result<(), String> {
        io.send(&uci_go_command(req))
    }

    fn start_ponder(&mut self, io: &mut EngineIo, req: &SearchRequest) -> Result<(), String> {
        io.send(&uci_ponder_command(req))
    }

    fn ponder_hit(&mut self, io: &mut EngineIo) -> Result<(), String> {
        io.send("ponderhit")
    }

    fn stop_search(&mut self, io: &mut EngineIo) -> Result<(), String> {
        io.send("stop")
    }

    fn parse_output_line(&mut self, line: &str, info: &mut SearchInfo) -> Option<String> {
        if line.starts_with("info ") {
            self.parse_info_line(line, info);
            return None;
        }
        if line.starts_with("bestmove ") {
            let mut tokens = line.split_whitespace();
            let _ = tokens.next();
            let best_move = tokens.next().map(ToOwned::to_owned);
            if tokens.next() == Some("ponder") {
                info.ponder_move = tokens.next().map(ToOwned::to_owned);
            }
            return best_move;
        }
        None
    }
}

#[derive(Default)]
struct XboardDriver;

impl XboardDriver {
    fn parse_post_line(&self, line: &str, info: &mut SearchInfo) {
        // Typical post format: "<depth> <score> <time> <nodes> <pv...>"
        let mut it = line.split_whitespace();
        let depth = it.next().and_then(|s| s.parse::<i32>().ok());
        let score = it.next().and_then(|s| s.parse::<i32>().ok());
        let time = it.next().and_then(|s| s.parse::<u64>().ok());
        let nodes = it.next().and_then(|s| s.parse::<u64>().ok());
        if let Some(d) = depth {
            info.depth = d;
        }
        if let Some(s) = score {
            info.score = s;
        }
        if let Some(n) = nodes {
            info.nodes = n;
        }
        if let Some(centiseconds) = time {
            info.time_ms = centiseconds.saturating_mul(10);
        }
        info.pv.clear();
        info.pv.extend(it.map(ToOwned::to_owned));
    }
}

impl ProtocolDriver for XboardDriver {
    fn initialize(&mut self, io: &mut EngineIo) -> Result<(), String> {
        io.send("xboard")?;
        io.send("protover 2")?;
        // Read feature lines until done=1 (or until engine starts normal output).
        for _ in 0..128 {
            let line = io.read_line()?;
            if line.starts_with("feature ") && line.contains("done=1") {
                break;
            }
            if line.is_empty() {
                continue;
            }
            if !line.starts_with("feature ") {
                break;
            }
        }
        io.send("new")?;
        Ok(())
    }

    fn configure(&mut self, io: &mut EngineIo, options: &EngineOptions) -> Result<(), String> {
        if let Some(hash_mb) = options.hash_mb {
            io.send(&format!("memory {hash_mb}"))?;
        }
        if let Some(threads) = options.threads {
            io.send(&format!("cores {threads}"))?;
        }
        Ok(())
    }

    fn new_game(&mut self, io: &mut EngineIo) -> Result<(), String> {
        io.send("new")
    }

    fn set_position(
        &mut self,
        io: &mut EngineIo,
        fen: &str,
        moves: &[String],
    ) -> Result<(), String> {
        io.send("force")?;
        io.send(&format!("setboard {fen}"))?;
        for mv in moves {
            io.send(&format!("usermove {mv}"))?;
        }
        Ok(())
    }

    fn start_search(&mut self, io: &mut EngineIo, req: &SearchRequest) -> Result<(), String> {
        io.send(&format!("sd {}", req.depth.max(1)))?;
        if let Some(movetime) = req.movetime {
            let secs = (movetime.as_millis() as f64 / 1000.0).ceil() as u64;
            io.send(&format!("st {}", secs.max(1)))?;
        }
        io.send("go")
    }

    fn parse_output_line(&mut self, line: &str, info: &mut SearchInfo) -> Option<String> {
        if let Some(rest) = line.strip_prefix("move ") {
            return rest.split_whitespace().next().map(ToString::to_string);
        }

        if line
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '-')
        {
            self.parse_post_line(line, info);
        }
        None
    }
}

fn uci_go_command(req: &SearchRequest) -> String {
    uci_go_command_with_prefix(req, "go")
}

fn uci_ponder_command(req: &SearchRequest) -> String {
    uci_go_command_with_prefix(req, "go ponder")
}

fn uci_go_command_with_prefix(req: &SearchRequest, prefix: &str) -> String {
    let mut cmd = String::from(prefix);
    if let Some(nodes) = req.node_limit {
        cmd.push_str(&format!(" nodes {}", nodes.max(1)));
        return cmd;
    } else if let Some(movetime) = req.movetime {
        cmd.push_str(&format!(" movetime {}", movetime.as_millis().max(1)));
        return cmd;
    }
    cmd.push_str(&format!(" depth {}", req.depth.max(1)));
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uci_info_line_parse_cp() {
        let drv = UciDriver;
        let mut info = SearchInfo::default();
        drv.parse_info_line(
            "info depth 12 seldepth 18 score cp 45 nodes 12345 nps 900000 time 27 hashfull 12 tbhits 3 currmove e2e4 currmovenumber 4 pv e2e4 e7e5",
            &mut info,
        );
        assert_eq!(info.depth, 12);
        assert_eq!(info.score, 45);
        assert_eq!(info.nodes, 12345);
        assert_eq!(info.nps, 900000);
        assert_eq!(info.seldepth, 18);
        assert_eq!(info.time_ms, 27);
        assert_eq!(info.hashfull, 12);
        assert_eq!(info.tablebase_hits, 3);
        assert_eq!(info.current_move.as_deref(), Some("e2e4"));
        assert_eq!(info.current_move_number, 4);
        assert_eq!(info.pv, ["e2e4", "e7e5"]);
    }

    #[test]
    fn test_uci_info_line_parse_mate() {
        let drv = UciDriver;
        let mut info = SearchInfo::default();
        drv.parse_info_line("info depth 10 score mate 3 nodes 1000", &mut info);
        assert_eq!(info.depth, 10);
        assert_eq!(info.score, 28_997);
        assert_eq!(info.nodes, 1000);
    }

    #[test]
    fn test_uci_bestmove_parse_ponder() {
        let mut driver = UciDriver;
        let mut info = SearchInfo::default();
        let best = driver.parse_output_line("bestmove e2e4 ponder e7e5", &mut info);
        assert_eq!(best.as_deref(), Some("e2e4"));
        assert_eq!(info.ponder_move.as_deref(), Some("e7e5"));
    }

    #[test]
    fn test_xboard_parse_post_line() {
        let drv = XboardDriver;
        let mut info = SearchInfo::default();
        drv.parse_post_line("14 36 1234 987654 e2e4 e7e5", &mut info);
        assert_eq!(info.depth, 14);
        assert_eq!(info.score, 36);
        assert_eq!(info.nodes, 987654);
    }

    #[test]
    fn test_protocol_display() {
        assert_eq!(ProtocolKind::Uci.to_string(), "uci");
        assert_eq!(ProtocolKind::Xboard.to_string(), "xboard");
    }

    #[test]
    fn custom_uci_option_command_supports_paths_with_spaces() {
        assert_eq!(
            uci_setoption_command("EvalFile", r"C:\Networks\Reckless v60.nnue").unwrap(),
            r"setoption name EvalFile value C:\Networks\Reckless v60.nnue"
        );
    }

    #[test]
    fn custom_uci_option_rejects_protocol_injection() {
        assert!(uci_setoption_command("EvalFile\nisready", "network.nnue").is_err());
        assert!(uci_setoption_command("EvalFile", "network.nnue\nquit").is_err());
    }

    #[test]
    fn uci_position_command_preserves_history() {
        let moves = vec!["e2e4".to_string(), "e7e5".to_string()];
        assert_eq!(
            uci_position_command("start-fen", &moves),
            "position fen start-fen moves e2e4 e7e5"
        );
        assert_eq!(
            uci_position_command("start-fen", &[]),
            "position fen start-fen"
        );
    }

    #[test]
    fn uci_initialization_starts_with_a_portable_no_op() {
        assert_eq!(uci_initialization_commands(), ["", "uci"]);
    }

    #[test]
    fn uci_go_command_emits_exactly_one_hard_limit() {
        let request = SearchRequest {
            fen: String::new(),
            moves: Vec::new(),
            depth: 64,
            movetime: Some(Duration::from_millis(250)),
            node_limit: Some(1_000),
        };

        assert_eq!(uci_go_command(&request), "go nodes 1000");

        let request = SearchRequest {
            node_limit: None,
            ..request
        };
        assert_eq!(uci_go_command(&request), "go movetime 250");

        let request = SearchRequest {
            movetime: None,
            ..request
        };
        assert_eq!(uci_go_command(&request), "go depth 64");
    }

    #[test]
    fn uci_ponder_command_preserves_the_selected_hard_limit() {
        let request = SearchRequest {
            fen: String::new(),
            moves: vec!["e2e4".to_owned(), "e7e5".to_owned()],
            depth: 32,
            movetime: None,
            node_limit: Some(50_000),
        };
        assert_eq!(uci_ponder_command(&request), "go ponder nodes 50000");

        let request = SearchRequest {
            node_limit: None,
            movetime: Some(Duration::from_millis(750)),
            ..request
        };
        assert_eq!(uci_ponder_command(&request), "go ponder movetime 750");
    }

    #[test]
    fn stderr_tail_is_bounded_and_utf8_safe() {
        let tail = Mutex::new(VecDeque::new());
        for index in 0..(STDERR_TAIL_LINES + 3) {
            record_stderr_line(&tail, &format!("line-{index}"));
        }
        record_stderr_line(&tail, &"é".repeat(STDERR_LINE_BYTES));

        let tail = tail.lock().unwrap();
        assert_eq!(tail.len(), STDERR_TAIL_LINES);
        assert_eq!(tail.front().unwrap(), "line-4");
        assert!(tail.back().unwrap().len() <= STDERR_LINE_BYTES);
        assert!(
            tail.back()
                .unwrap()
                .is_char_boundary(tail.back().unwrap().len())
        );
        drop(tail);
    }

    #[test]
    fn dropping_io_does_not_wait_forever_for_unresponsive_child() {
        const CHILD_MARKER: &str = "MUJRIM_PROTOCOL_DROP_TEST_CHILD";
        if std::env::var_os(CHILD_MARKER).is_some() {
            std::thread::sleep(Duration::from_secs(5));
            return;
        }

        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::dropping_io_does_not_wait_forever_for_unresponsive_child",
            ])
            .env(CHILD_MARKER, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let kill_on_close = process_safety::KillOnCloseJob::attach(&child, None).unwrap();
        let stdin = child.stdin.take().unwrap();
        let (_tx, rx) = mpsc::channel();
        let io = EngineIo {
            child,
            _kill_on_close: kill_on_close,
            stdin,
            stdout_rx: rx,
            stderr_tail: Arc::new(Mutex::new(VecDeque::new())),
            read_timeout: DEFAULT_READ_TIMEOUT,
            memory_limit_bytes: None,
        };
        let start = Instant::now();
        drop(io);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(windows)]
    #[test]
    fn engine_io_enforces_working_set_limit() {
        let args = vec!["/C".to_string(), "ping -n 30 127.0.0.1 >NUL".to_string()];
        let mut io =
            EngineIo::spawn_bounded(Path::new("cmd.exe"), &args, None).expect("spawn test child");
        io.set_memory_limit_bytes(Some(1));

        let error = io.read_line().expect_err("one-byte limit must stop child");

        assert!(error.contains("working set exceeded limit"));
        assert!(io.child.try_wait().expect("query child").is_some());
    }

    #[test]
    fn passthrough_reports_spawn_errors_without_panicking() {
        let missing = std::env::temp_dir().join("mujrim-engine-that-does-not-exist");
        let error = run_passthrough_with_memory_limit(&missing, &[], Some(1024))
            .expect_err("missing passthrough engine must fail");
        assert!(error.contains("failed to start"));
    }

    #[test]
    fn passthrough_forwards_recursion_guard_environment() {
        const MARKER: &str = "MUJRIM_PASSTHROUGH_ENV_TEST_CHILD";
        if std::env::var_os(MARKER).is_some() {
            return;
        }

        let executable = std::env::current_exe().expect("locate test executable");
        let args = vec![
            "--exact".to_owned(),
            "tests::passthrough_forwards_recursion_guard_environment".to_owned(),
        ];
        let status = run_passthrough_with_environment(&executable, &args, &[(MARKER, "1")], None)
            .expect("run child test process");
        assert!(status.success());
    }

    #[test]
    fn identity_adapter_preserves_search_telemetry_and_bestmove() {
        let input = b"id name Stockfish dev\r\nid author the Stockfish developers\r\noption name Hash type spin default 16 min 1 max 33554432\r\nuciok\r\ninfo depth 18 score cp 31 nodes 500000 nps 900000 pv e2e4 e7e5\r\nbestmove e2e4 ponder e7e5\r\n";
        let adapter = UciIdentityAdapter {
            name: "Mujrim Elite 2.0.0 [Stockfish-native]",
            author: "Ahmad Hamdi Emara (Egypt) / Stockfish developers",
        };
        let mut output = Vec::new();

        relay_uci_output(BufReader::new(&input[..]), &mut output, &adapter).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("id name Mujrim Elite 2.0.0 [Stockfish-native]\r\n"));
        assert!(output.contains("id author Ahmad Hamdi Emara (Egypt) / Stockfish developers\r\n"));
        assert!(output.contains("option name Hash type spin default 16 min 1 max 33554432\r\n"));
        assert!(
            output.contains("info depth 18 score cp 31 nodes 500000 nps 900000 pv e2e4 e7e5\r\n")
        );
        assert!(output.ends_with("bestmove e2e4 ponder e7e5\r\n"));
    }

    #[test]
    fn bounded_adapter_clamps_allocating_options_before_the_backend() {
        let adapter = BoundedUciIdentityAdapter {
            identity: UciIdentityAdapter {
                name: "Mujrim Elite",
                author: "Mujrim",
            },
            max_hash_mb: 1024,
            max_threads: 12,
        };
        let input = b"setoption name Hash value 65536\r\nsetoption name Threads value 512\r\nposition startpos\r\ngo nodes 100000\r\n";
        let mut output = Vec::new();

        relay_uci_input(BufReader::new(&input[..]), &mut output, &adapter).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "setoption name Hash value 1024\r\nsetoption name Threads value 12\r\nposition startpos\r\ngo nodes 100000\r\n"
        );
    }

    #[test]
    fn bounded_adapter_advertises_real_limits_without_touching_nps() {
        let adapter = BoundedUciIdentityAdapter {
            identity: UciIdentityAdapter {
                name: "Mujrim Elite",
                author: "Mujrim",
            },
            max_hash_mb: 1024,
            max_threads: 12,
        };
        let input = b"option name Hash type spin default 16 min 1 max 33554432\noption name Threads type spin default 1 min 1 max 1024\ninfo depth 20 nodes 1000000 nps 900000 pv e2e4\nbestmove e2e4\n";
        let mut output = Vec::new();

        relay_uci_output(BufReader::new(&input[..]), &mut output, &adapter).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("option name Hash type spin default 16 min 1 max 1024\n"));
        assert!(output.contains("option name Threads type spin default 1 min 1 max 12\n"));
        assert!(output.contains("info depth 20 nodes 1000000 nps 900000 pv e2e4\n"));
        assert!(output.ends_with("bestmove e2e4\n"));
    }
}
