//! Host-side limits for a candidate process.
//!
//! Lust's sandbox has no time limit. `--max-memory` (lust 0.3.0) is checked
//! inside the guest when it allocates, and it does not cover a reservation
//! that is touched later without a new allocation, nor the stack. The host
//! therefore always has its own wall-clock kill and its own memory backstop:
//! `RLIMIT_AS` on Unix, a job object on Windows.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{err, Result};

#[derive(Clone, Debug)]
pub struct Limits {
    pub wall: Duration,
    /// Address-space cap (Unix) or process commit cap (Windows).
    pub memory_bytes: Option<u64>,
    /// Drop the parent environment before exec. On for candidate programs,
    /// off for the trusted `lust` and `rustc` tools.
    pub scrub_env: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopKind {
    Exited(i32),
    TimedOut,
    /// The process died by signal while a host memory cap was installed.
    MemorySignal,
}

#[derive(Debug)]
pub struct ExecResult {
    pub kind: StopKind,
    pub stdout: String,
    pub stderr: String,
    pub elapsed: Duration,
}

impl ExecResult {
    pub fn success(&self) -> bool {
        matches!(self.kind, StopKind::Exited(0))
    }
}

/// Run `cmd` to completion. `input` is written after the memory cap is in
/// place. Stdout and stderr are capped.
pub fn run_capture(mut cmd: Command, limits: &Limits, input: &[u8]) -> Result<ExecResult> {
    prepare(&mut cmd, limits);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let _job = apply_windows_job(&child, limits)?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| err("stdin was not piped"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| err("stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| err("stderr was not piped"))?;
    let input = input.to_vec();
    // Not joined: a grandchild that inherited stdin can keep the write
    // blocked after the child is gone.
    thread::spawn(move || {
        let _ = stdin.write_all(&input);
        let _ = stdin.flush();
    });
    let stdout_rx = spawn_reader(stdout);
    let stderr_rx = spawn_reader(stderr);
    let start = Instant::now();
    let kind = wait_child(&mut child, limits, start)?;
    // The child is gone. A grandchild may still hold the pipes, so the
    // readers get a short grace period and are never joined.
    let deadline = Instant::now() + DRAIN_GRACE;
    let stdout = bytes_to_string(&collect(&stdout_rx, deadline));
    let stderr = bytes_to_string(&collect(&stderr_rx, deadline));
    Ok(ExecResult {
        kind,
        stdout,
        stderr,
        elapsed: start.elapsed(),
    })
}

/// Interactive line protocol. One request line, one response line.
pub struct Session {
    child: Option<Child>,
    stdin: Option<std::process::ChildStdin>,
    lines: Receiver<Option<String>>,
    stderr_rx: Option<Receiver<Vec<u8>>>,
    stderr: String,
    start: Instant,
    wall: Duration,
    memory_limited: bool,
    finished: Option<StopKind>,
    _job: Option<JobGuard>,
}

impl Session {
    pub fn spawn(mut cmd: Command, limits: &Limits) -> Result<Self> {
        prepare(&mut cmd, limits);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        let job = apply_windows_job(&child, limits)?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| err("stdin was not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| err("stdout was not piped"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| err("stderr was not piped"))?;
        let (tx, rx) = mpsc::channel();
        // Not joined, like the readers in `spawn_reader`.
        thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match std::io::BufRead::read_line(&mut reader, &mut line) {
                    Ok(0) => {
                        let _ = tx.send(None);
                        break;
                    }
                    Ok(_) => {
                        if tx.send(Some(line)).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(None);
                        break;
                    }
                }
            }
        });
        let stderr_rx = spawn_reader(stderr);
        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            lines: rx,
            stderr_rx: Some(stderr_rx),
            stderr: String::new(),
            start: Instant::now(),
            wall: limits.wall,
            memory_limited: limits.memory_bytes.is_some(),
            finished: None,
            _job: job,
        })
    }

    pub fn exchange(&mut self, line: &str) -> Result<String> {
        if self.finished.is_some() {
            return Err(err("the candidate has already stopped"));
        }
        if self.start.elapsed() > self.wall {
            self.stop(StopKind::TimedOut);
            return Err(err("candidate exceeded the wall-clock limit"));
        }
        {
            let stdin = self.stdin.as_mut().ok_or_else(|| err("stdin closed"))?;
            writeln!(stdin, "{line}")?;
            stdin.flush()?;
        }
        let remaining = self.wall.saturating_sub(self.start.elapsed());
        match self.lines.recv_timeout(remaining) {
            Ok(Some(line)) => Ok(line),
            Ok(None) => {
                self.finish_exit()?;
                Err(err("candidate closed stdout before answering"))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.stop(StopKind::TimedOut);
                Err(err("candidate exceeded the wall-clock limit"))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.finish_exit()?;
                Err(err("candidate closed stdout before answering"))
            }
        }
    }

    pub fn finish(mut self) -> Result<ExecResult> {
        if let Some(stdin) = self.stdin.take() {
            drop(stdin);
        }
        let start = self.start;
        let wall = self.wall;
        let kind = if let Some(kind) = self.finished {
            kind
        } else if let Some(child) = self.child.as_mut() {
            let grace = wall.saturating_add(Duration::from_secs(2));
            let kind = wait_child(
                child,
                &Limits {
                    wall: grace,
                    memory_bytes: None,
                    scrub_env: false,
                },
                start,
            )?;
            self.classify_wait(kind)
        } else {
            StopKind::Exited(1)
        };
        self.finished = Some(kind);
        let stdout = String::new();
        let stderr = self.take_stderr();
        let elapsed = start.elapsed();
        // Prevent Drop from killing a process that already exited.
        self.child = None;
        Ok(ExecResult {
            kind,
            stdout,
            stderr,
            elapsed,
        })
    }

    fn classify_wait(&self, kind: StopKind) -> StopKind {
        match kind {
            StopKind::MemorySignal if !self.memory_limited => StopKind::Exited(1),
            other => other,
        }
    }

    fn finish_exit(&mut self) -> Result<()> {
        if self.finished.is_some() {
            return Ok(());
        }
        if let Some(child) = self.child.as_mut() {
            let polled = child.try_wait()?;
            if let Some(status) = polled {
                self.finished = Some(status_kind(status, self.memory_limited));
            }
        }
        Ok(())
    }

    fn stop(&mut self, kind: StopKind) {
        if let Some(child) = self.child.as_mut() {
            kill_tree(child);
            let _ = child.wait();
        }
        self.finished = Some(kind);
    }

    fn take_stderr(&mut self) -> String {
        // The child has stopped; a grandchild may still hold the pipe.
        if let Some(rx) = self.stderr_rx.take() {
            self.stderr = bytes_to_string(&collect(&rx, Instant::now() + DRAIN_GRACE));
        }
        self.stderr.clone()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            kill_tree(child);
            let _ = child.wait();
        }
        // The reader threads are detached; dropping the receivers lets them
        // stop at their next chunk or at end-of-file.
    }
}

fn prepare(cmd: &mut Command, limits: &Limits) {
    if limits.scrub_env {
        harden_env(cmd);
    }
    #[cfg(unix)]
    {
        let bytes = limits.memory_bytes;
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(move || {
                set_process_group()?;
                if let Some(bytes) = bytes {
                    set_address_limit(bytes)?;
                }
                Ok(())
            });
        }
    }
    // Every child here gets pipes for its three standard streams. On Windows
    // it would also get a copy of every other inheritable handle of this
    // process, this process's own stdin, stdout and stderr among them, and
    // would pass them on to whatever it starts. A background process two
    // levels down (lust's daemon) then keeps this process's stdout open, and
    // a caller reading it never sees end-of-file.
    #[cfg(windows)]
    windows::keep_std_handles_private();
    let _ = limits;
}

fn harden_env(cmd: &mut Command) {
    #[cfg(unix)]
    {
        let path = std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into());
        cmd.env_clear();
        cmd.env("PATH", path);
    }
    #[cfg(windows)]
    {
        for (key, _) in std::env::vars() {
            let upper = key.to_ascii_uppercase();
            if upper.contains("SECRET")
                || upper.contains("TOKEN")
                || upper.contains("PASSWORD")
                || upper.contains("CREDENTIAL")
                || upper.contains("API_KEY")
                || upper.ends_with("_KEY")
            {
                cmd.env_remove(&key);
            }
        }
    }
}

fn wait_child(child: &mut Child, limits: &Limits, start: Instant) -> Result<StopKind> {
    loop {
        if start.elapsed() > limits.wall {
            kill_tree(child);
            let _ = child.wait();
            return Ok(StopKind::TimedOut);
        }
        if let Some(status) = child.try_wait()? {
            return Ok(status_kind(status, limits.memory_bytes.is_some()));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn status_kind(status: std::process::ExitStatus, memory_limited: bool) -> StopKind {
    if let Some(code) = status.code() {
        return StopKind::Exited(code);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if memory_limited {
            if let Some(signal) = status.signal() {
                // SIGKILL, SIGSEGV, SIGABRT, SIGBUS: what a tight address cap produces.
                if matches!(signal, 9 | 11 | 6 | 7) {
                    return StopKind::MemorySignal;
                }
            }
        }
    }
    let _ = memory_limited;
    StopKind::Exited(1)
}

fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        unsafe {
            // The child called setpgid(0, 0), so its pid is the group id.
            extern "C" {
                fn kill(pid: i32, sig: i32) -> i32;
            }
            let _ = kill(-pid, 9);
        }
    }
    let _ = child.kill();
}

/// How long the readers may keep draining after the child has stopped.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

/// Read `reader` on a detached thread and send each chunk as it arrives.
/// The channel disconnects at end-of-file, on error, or at the 2 MiB cap.
/// The thread is never joined, so a pipe held open by a grandchild cannot
/// block the caller.
fn spawn_reader<R>(mut reader: R) -> Receiver<Vec<u8>>
where
    R: Read + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut chunk = [0u8; 8 * 1024];
        let cap: usize = 2 * 1024 * 1024;
        let mut total = 0usize;
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let room = cap.saturating_sub(total);
                    if room == 0 {
                        break;
                    }
                    let take = n.min(room);
                    total += take;
                    if tx.send(chunk[..take].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    rx
}

/// Gather what a reader has sent, until it disconnects or `deadline` passes.
/// Anything already queued at the deadline is still taken.
fn collect(rx: &Receiver<Vec<u8>>, deadline: Instant) -> Vec<u8> {
    let mut buf = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(chunk) => buf.extend_from_slice(&chunk),
            Err(mpsc::RecvTimeoutError::Disconnected) => return buf,
            Err(mpsc::RecvTimeoutError::Timeout) => break,
        }
    }
    while let Ok(chunk) = rx.try_recv() {
        buf.extend_from_slice(&chunk);
    }
    buf
}

fn bytes_to_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(unix)]
fn set_process_group() -> std::io::Result<()> {
    extern "C" {
        fn setpgid(pid: i32, pgid: i32) -> i32;
    }
    let rc = unsafe { setpgid(0, 0) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn set_address_limit(bytes: u64) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    const RLIMIT_AS: i32 = 9;
    #[cfg(target_os = "macos")]
    const RLIMIT_AS: i32 = 5;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    const RLIMIT_AS: i32 = 9;

    #[repr(C)]
    struct RLimit {
        cur: u64,
        max: u64,
    }
    extern "C" {
        fn setrlimit(resource: i32, rlim: *const RLimit) -> i32;
    }
    let limit = RLimit {
        cur: bytes,
        max: bytes,
    };
    let rc = unsafe { setrlimit(RLIMIT_AS, &limit) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

struct JobGuard {
    #[cfg(windows)]
    raw: *mut std::ffi::c_void,
}

#[cfg(windows)]
unsafe impl Send for JobGuard {}

impl Drop for JobGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            if !self.raw.is_null() {
                windows::close_handle(self.raw);
            }
        }
    }
}

#[cfg(windows)]
fn apply_windows_job(child: &Child, limits: &Limits) -> Result<Option<JobGuard>> {
    let Some(bytes) = limits.memory_bytes else {
        return Ok(None);
    };
    let raw = windows::assign_memory_job(child, bytes)?;
    Ok(Some(JobGuard { raw }))
}

#[cfg(not(windows))]
fn apply_windows_job(_child: &Child, _limits: &Limits) -> Result<Option<JobGuard>> {
    Ok(None)
}

#[cfg(windows)]
mod windows {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    use crate::error::{err, Result};

    type HANDLE = *mut std::ffi::c_void;
    type BOOL = i32;
    type DWORD = u32;

    const JOB_OBJECT_LIMIT_PROCESS_MEMORY: DWORD = 0x0000_0100;
    const JOBOBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;

    #[repr(C)]
    struct BasicLimit {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: DWORD,
        _pad: DWORD,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: DWORD,
        _pad2: DWORD,
        affinity: usize,
        priority_class: DWORD,
        scheduling_class: DWORD,
    }

    #[repr(C)]
    struct ExtendedLimit {
        basic: BasicLimit,
        io: [u64; 6],
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    extern "system" {
        fn CreateJobObjectW(attrs: *mut u8, name: *const u16) -> HANDLE;
        fn SetInformationJobObject(job: HANDLE, class: i32, info: *const u8, len: DWORD) -> BOOL;
        fn AssignProcessToJobObject(job: HANDLE, process: HANDLE) -> BOOL;
        fn CloseHandle(handle: HANDLE) -> BOOL;
        fn GetStdHandle(id: DWORD) -> HANDLE;
        fn SetHandleInformation(handle: HANDLE, mask: DWORD, flags: DWORD) -> BOOL;
    }

    #[cfg(test)]
    extern "system" {
        fn GetHandleInformation(handle: HANDLE, flags: *mut DWORD) -> BOOL;
    }

    const HANDLE_FLAG_INHERIT: DWORD = 1;
    // STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE.
    const STD_HANDLES: [i32; 3] = [-10, -11, -12];

    /// Make this process's stdin, stdout and stderr non-inheritable, once.
    /// A child that should share them still can: `Stdio::inherit()` hands
    /// the child its own inheritable duplicate.
    pub fn keep_std_handles_private() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            for id in STD_HANDLES {
                // SAFETY: plain Win32 calls on this process's own handles. A
                // missing or invalid handle fails the call and is left alone.
                unsafe {
                    let handle = GetStdHandle(id as DWORD);
                    if !handle.is_null() && handle as isize != -1 {
                        SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
                    }
                }
            }
        });
    }

    /// Whether any of this process's standard handles is still inheritable.
    #[cfg(test)]
    pub fn a_std_handle_is_inheritable() -> bool {
        STD_HANDLES.iter().any(|&id| {
            let mut flags: DWORD = 0;
            // SAFETY: as above; a failed query leaves `flags` at 0.
            unsafe {
                let handle = GetStdHandle(id as DWORD);
                !handle.is_null()
                    && handle as isize != -1
                    && GetHandleInformation(handle, &mut flags) != 0
                    && flags & HANDLE_FLAG_INHERIT != 0
            }
        })
    }

    pub fn close_handle(handle: *mut std::ffi::c_void) {
        unsafe { CloseHandle(handle) };
    }

    pub fn assign_memory_job(child: &Child, bytes: u64) -> Result<*mut std::ffi::c_void> {
        const _: () = assert!(std::mem::size_of::<BasicLimit>() == 64);
        const _: () = assert!(std::mem::size_of::<ExtendedLimit>() == 144);
        unsafe {
            let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
            if job.is_null() {
                return Err(err("CreateJobObjectW failed"));
            }
            let mut info = ExtendedLimit {
                basic: BasicLimit {
                    per_process_user_time_limit: 0,
                    per_job_user_time_limit: 0,
                    limit_flags: JOB_OBJECT_LIMIT_PROCESS_MEMORY,
                    _pad: 0,
                    minimum_working_set_size: 0,
                    maximum_working_set_size: 0,
                    active_process_limit: 0,
                    _pad2: 0,
                    affinity: 0,
                    priority_class: 0,
                    scheduling_class: 0,
                },
                io: [0; 6],
                process_memory_limit: bytes as usize,
                job_memory_limit: 0,
                peak_process_memory_used: 0,
                peak_job_memory_used: 0,
            };
            let ok = SetInformationJobObject(
                job,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                &mut info as *mut ExtendedLimit as *const u8,
                std::mem::size_of::<ExtendedLimit>() as DWORD,
            );
            if ok == 0 {
                CloseHandle(job);
                return Err(err("SetInformationJobObject failed"));
            }
            let assigned = AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE);
            if assigned == 0 {
                CloseHandle(job);
                return Err(err(
                    "AssignProcessToJobObject failed; the host memory cap could not be installed",
                ));
            }
            Ok(job)
        }
    }
}

/// Parse `64K`, `128M`, `1G`, or a bare byte count. Binary units, matching lust.
pub fn parse_memory(text: &str) -> Result<u64> {
    let text = text.trim();
    if text.is_empty() {
        return Err(err("empty memory size"));
    }
    let (number, factor) = if let Some(rest) = text.strip_suffix(['K', 'k']) {
        (rest, 1024u64)
    } else if let Some(rest) = text.strip_suffix(['M', 'm']) {
        (rest, 1024 * 1024)
    } else if let Some(rest) = text.strip_suffix(['G', 'g']) {
        (rest, 1024 * 1024 * 1024)
    } else {
        (text, 1u64)
    };
    let value: u64 = number
        .trim()
        .parse()
        .map_err(|_| err(format!("could not parse memory size `{text}`")))?;
    value
        .checked_mul(factor)
        .ok_or_else(|| err(format!("memory size `{text}` overflows")))
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn spawning_a_child_makes_the_std_handles_private() {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", "exit 0"]);
        let limits = super::Limits {
            wall: std::time::Duration::from_secs(20),
            memory_bytes: None,
            scrub_env: false,
        };
        let result = super::run_capture(cmd, &limits, b"").unwrap();
        assert!(result.success());
        assert!(!super::windows::a_std_handle_is_inheritable());
    }

    use super::*;

    #[test]
    fn parses_lust_memory_syntax() {
        assert_eq!(parse_memory("4096").unwrap(), 4096);
        assert_eq!(parse_memory("64K").unwrap(), 64 * 1024);
        assert_eq!(parse_memory("128M").unwrap(), 128 * 1024 * 1024);
        assert_eq!(parse_memory("1G").unwrap(), 1024 * 1024 * 1024);
        assert!(parse_memory("nope").is_err());
    }
}
