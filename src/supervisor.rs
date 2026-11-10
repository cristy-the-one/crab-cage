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
use std::thread::{self, JoinHandle};
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
    let writer = thread::spawn(move || {
        let _ = stdin.write_all(&input);
        let _ = stdin.flush();
    });
    let stdout_thread = spawn_reader(stdout);
    let stderr_thread = spawn_reader(stderr);
    let start = Instant::now();
    let kind = wait_child(&mut child, limits, start)?;
    let _ = writer.join();
    let stdout = join_text(stdout_thread);
    let stderr = join_text(stderr_thread);
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
    reader: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<Vec<u8>>>,
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
        let reader = thread::spawn(move || {
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
        let stderr_thread = spawn_reader(stderr);
        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            lines: rx,
            reader: Some(reader),
            stderr_thread: Some(stderr_thread),
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
        if let Some(handle) = self.stderr_thread.take() {
            self.stderr = bytes_to_string(&join_bytes(handle));
        }
        if let Some(handle) = self.reader.take() {
            let _ = handle.join();
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
        if let Some(handle) = self.stderr_thread.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.reader.take() {
            let _ = handle.join();
        }
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

fn spawn_reader<R>(mut reader: R) -> JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8 * 1024];
        let cap: usize = 2 * 1024 * 1024;
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let room = cap.saturating_sub(buf.len());
                    if room == 0 {
                        break;
                    }
                    let take = n.min(room);
                    buf.extend_from_slice(&chunk[..take]);
                }
                Err(_) => break,
            }
        }
        buf
    })
}

fn join_text(handle: JoinHandle<Vec<u8>>) -> String {
    bytes_to_string(&join_bytes(handle))
}

fn join_bytes(handle: JoinHandle<Vec<u8>>) -> Vec<u8> {
    handle.join().unwrap_or_default()
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
                unsafe { windows::close_handle(self.raw) };
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
