//! Starting a child process with a pipe on each of its three standard streams.
//!
//! On Unix this is `std::process::Command`. On Windows it is a direct
//! `CreateProcessW` call, for two reasons.
//!
//! - **Inheritance.** A Windows child gets a copy of every inheritable handle
//!   of its parent, whatever its own standard handles are. This process's own
//!   stdin, stdout and stderr are inheritable, so a child got them too and
//!   passed them on; a background process two levels down (lust's daemon)
//!   then kept this process's stdout open. A handle list
//!   (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`) names the only handles the child
//!   gets: here, its three pipe ends.
//! - **The lock.** `Command::spawn` guards against the same leak with one
//!   process-wide lock held across `CreateProcessW`. The first start of an
//!   executable that has never run takes about 40 ms on Windows, every
//!   candidate is such an executable, and under that lock those starts never
//!   overlap. With a handle list no lock is needed.
//!
//! std has the handle list only behind an unstable feature, so the call is
//! made here. A `.cmd` or `.bat` program still goes through `Command`, which
//! knows how to hand it to `cmd.exe`.

use std::io::{self, Read, Write};
use std::process::Command;

pub(crate) type ChildIn = Box<dyn Write + Send>;
pub(crate) type ChildOut = Box<dyn Read + Send>;

/// How a child ended: an exit code, or on Unix the signal that killed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Exit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

/// A started child and this process's ends of its three pipes.
pub(crate) struct Piped {
    pub proc: Proc,
    pub stdin: ChildIn,
    pub stdout: ChildOut,
    pub stderr: ChildOut,
}

/// A child process. Dropping it does not kill or wait for the child.
pub(crate) enum Proc {
    Std(std::process::Child),
    #[cfg(windows)]
    Raw(raw::Process),
}

impl Proc {
    /// The process id, which on Unix is also its process group.
    #[cfg(unix)]
    pub fn id(&self) -> u32 {
        match self {
            Proc::Std(child) => child.id(),
        }
    }

    /// `None` while the child is still running.
    pub fn try_wait(&mut self) -> io::Result<Option<Exit>> {
        match self {
            Proc::Std(child) => Ok(child.try_wait()?.map(exit_of)),
            #[cfg(windows)]
            Proc::Raw(process) => process.try_wait(),
        }
    }

    pub fn wait(&mut self) -> io::Result<Exit> {
        match self {
            Proc::Std(child) => child.wait().map(exit_of),
            #[cfg(windows)]
            Proc::Raw(process) => process.wait(),
        }
    }

    /// Ends the child. A child that has already exited is not an error.
    pub fn kill(&mut self) -> io::Result<()> {
        match self {
            Proc::Std(child) => child.kill(),
            #[cfg(windows)]
            Proc::Raw(process) => process.kill(),
        }
    }

    /// The process handle, for a job object.
    #[cfg(windows)]
    pub fn raw_handle(&self) -> *mut std::ffi::c_void {
        use std::os::windows::io::AsRawHandle;
        match self {
            Proc::Std(child) => child.as_raw_handle() as *mut std::ffi::c_void,
            Proc::Raw(process) => process.handle(),
        }
    }
}

fn exit_of(status: std::process::ExitStatus) -> Exit {
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal = None;
    Exit {
        code: status.code(),
        signal,
    }
}

/// Start `cmd` with a pipe on stdin, stdout and stderr. The program, its
/// arguments, the environment changes and the working directory are taken
/// from `cmd`; whatever it says about stdio is replaced.
pub(crate) fn spawn_piped(cmd: Command) -> io::Result<Piped> {
    #[cfg(windows)]
    {
        if !raw::is_batch_file(cmd.get_program()) {
            return raw::spawn(&cmd);
        }
        // `Command` is the fallback, and it lets a child inherit every
        // inheritable handle. Keep this process's own three out of that.
        raw::keep_std_handles_private();
    }
    spawn_std(cmd)
}

fn spawn_std(mut cmd: Command) -> io::Result<Piped> {
    use std::process::Stdio;
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let missing = |name: &str| io::Error::other(format!("{name} was not piped"));
    let stdin = child.stdin.take().ok_or_else(|| missing("stdin"))?;
    let stdout = child.stdout.take().ok_or_else(|| missing("stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| missing("stderr"))?;
    Ok(Piped {
        proc: Proc::Std(child),
        stdin: Box::new(stdin),
        stdout: Box::new(stdout),
        stderr: Box::new(stderr),
    })
}

#[cfg(windows)]
pub(crate) mod raw {
    use std::ffi::{c_void, OsStr, OsString};
    use std::fs::File;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::process::Command;

    use super::{Exit, Piped, Proc};

    type Handle = *mut c_void;

    #[repr(C)]
    struct StartupInfo {
        cb: u32,
        reserved: *mut u16,
        desktop: *mut u16,
        title: *mut u16,
        x: u32,
        y: u32,
        x_size: u32,
        y_size: u32,
        x_count_chars: u32,
        y_count_chars: u32,
        fill_attribute: u32,
        flags: u32,
        show_window: u16,
        cb_reserved2: u16,
        reserved2: *mut u8,
        std_input: Handle,
        std_output: Handle,
        std_error: Handle,
    }

    #[repr(C)]
    struct StartupInfoEx {
        startup: StartupInfo,
        attribute_list: *mut c_void,
    }

    #[repr(C)]
    struct ProcessInformation {
        process: Handle,
        thread: Handle,
        process_id: u32,
        thread_id: u32,
    }

    extern "system" {
        fn CreatePipe(
            read: *mut Handle,
            write: *mut Handle,
            attributes: *const c_void,
            size: u32,
        ) -> i32;
        fn SetHandleInformation(handle: Handle, mask: u32, flags: u32) -> i32;
        fn GetStdHandle(id: u32) -> Handle;
        fn InitializeProcThreadAttributeList(
            list: *mut c_void,
            count: u32,
            flags: u32,
            size: *mut usize,
        ) -> i32;
        fn UpdateProcThreadAttribute(
            list: *mut c_void,
            flags: u32,
            attribute: usize,
            value: *const c_void,
            size: usize,
            previous: *mut c_void,
            return_size: *mut usize,
        ) -> i32;
        fn DeleteProcThreadAttributeList(list: *mut c_void);
        fn CreateProcessW(
            application: *const u16,
            command_line: *mut u16,
            process_attributes: *const c_void,
            thread_attributes: *const c_void,
            inherit_handles: i32,
            creation_flags: u32,
            environment: *const c_void,
            current_directory: *const u16,
            startup_info: *const StartupInfo,
            process_information: *mut ProcessInformation,
        ) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
        fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
        fn GetExitCodeProcess(handle: Handle, code: *mut u32) -> i32;
        fn TerminateProcess(handle: Handle, code: u32) -> i32;
    }

    const HANDLE_FLAG_INHERIT: u32 = 1;
    const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;
    const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
    const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
    const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_TIMEOUT: u32 = 0x0000_0102;
    const INFINITE: u32 = 0xFFFF_FFFF;

    /// A process started by [`spawn`]. The handle is closed on drop; the
    /// process itself is left alone.
    pub(crate) struct Process {
        handle: OwnedHandle,
    }

    impl Process {
        pub fn handle(&self) -> Handle {
            self.handle.as_raw_handle() as Handle
        }

        pub fn try_wait(&mut self) -> io::Result<Option<Exit>> {
            self.wait_for(0)
        }

        pub fn wait(&mut self) -> io::Result<Exit> {
            match self.wait_for(INFINITE)? {
                Some(exit) => Ok(exit),
                None => Err(io::Error::other("the wait ended with the process running")),
            }
        }

        fn wait_for(&mut self, milliseconds: u32) -> io::Result<Option<Exit>> {
            // SAFETY: the handle is this struct's own open process handle.
            unsafe {
                match WaitForSingleObject(self.handle(), milliseconds) {
                    WAIT_OBJECT_0 => {
                        let mut code = 0u32;
                        if GetExitCodeProcess(self.handle(), &mut code) == 0 {
                            return Err(io::Error::last_os_error());
                        }
                        Ok(Some(Exit {
                            code: Some(code as i32),
                            signal: None,
                        }))
                    }
                    WAIT_TIMEOUT => Ok(None),
                    _ => Err(io::Error::last_os_error()),
                }
            }
        }

        pub fn kill(&mut self) -> io::Result<()> {
            // SAFETY: as above.
            let ended = unsafe { TerminateProcess(self.handle(), 1) };
            if ended != 0 || self.try_wait()?.is_some() {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
    }

    /// `.cmd` and `.bat` are not programs `CreateProcessW` can start.
    pub(crate) fn is_batch_file(program: &OsStr) -> bool {
        std::path::Path::new(program)
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
    }

    /// Make this process's stdin, stdout and stderr non-inheritable, once.
    pub(crate) fn keep_std_handles_private() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            // STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE.
            for id in [-10i32, -11, -12] {
                // SAFETY: plain Win32 calls on this process's own handles. A
                // missing or invalid handle fails the call and is left alone.
                unsafe {
                    let handle = GetStdHandle(id as u32);
                    if !handle.is_null() && handle as isize != -1 {
                        SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
                    }
                }
            }
        });
    }

    /// An anonymous pipe: `(read end, write end)`, neither inheritable.
    fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
        let mut read: Handle = std::ptr::null_mut();
        let mut write: Handle = std::ptr::null_mut();
        // SAFETY: both out-pointers are live locals. On success each handle
        // is owned exactly once, by the OwnedHandle made from it.
        unsafe {
            if CreatePipe(&mut read, &mut write, std::ptr::null(), 0) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok((
                OwnedHandle::from_raw_handle(read as _),
                OwnedHandle::from_raw_handle(write as _),
            ))
        }
    }

    fn inheritable(handle: &OwnedHandle) -> io::Result<()> {
        // SAFETY: the handle is open and owned by the caller.
        let done = unsafe {
            SetHandleInformation(
                handle.as_raw_handle() as Handle,
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            )
        };
        if done == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// One argument, quoted the way the C runtime's parser reads it back:
    /// quotes around an empty argument or one with a space or tab; a `"`
    /// gets a backslash; backslashes are doubled only in front of a `"`.
    fn append_arg(line: &mut Vec<u16>, arg: &OsStr, force_quotes: bool) {
        let units: Vec<u16> = arg.encode_wide().collect();
        let quote = force_quotes
            || units.is_empty()
            || units
                .iter()
                .any(|&unit| unit == u16::from(b' ') || unit == u16::from(b'\t'));
        if quote {
            line.push(u16::from(b'"'));
        }
        let mut backslashes = 0usize;
        for &unit in &units {
            if unit == u16::from(b'\\') {
                backslashes += 1;
            } else {
                if unit == u16::from(b'"') {
                    // The backslashes already pushed, once more, plus one
                    // for the quote itself.
                    line.extend(std::iter::repeat(u16::from(b'\\')).take(backslashes + 1));
                }
                backslashes = 0;
            }
            line.push(unit);
        }
        if quote {
            line.extend(std::iter::repeat(u16::from(b'\\')).take(backslashes));
            line.push(u16::from(b'"'));
        }
    }

    /// The command line for `program` and `args`, NUL-terminated.
    pub(crate) fn command_line<'a>(
        program: &OsStr,
        args: impl Iterator<Item = &'a OsStr>,
    ) -> Vec<u16> {
        let mut line = Vec::new();
        append_arg(&mut line, program, true);
        for arg in args {
            line.push(u16::from(b' '));
            append_arg(&mut line, arg, false);
        }
        line.push(0);
        line
    }

    /// This process's environment with `changes` applied (`None` removes a
    /// name), as a UTF-16 block. Names compare without case on Windows.
    pub(crate) fn environment_block<'a>(
        changes: impl Iterator<Item = (&'a OsStr, Option<&'a OsStr>)>,
    ) -> Vec<u16> {
        let changes: Vec<(&OsStr, Option<&OsStr>)> = changes.collect();
        let same = |a: &OsStr, b: &OsStr| {
            a.to_string_lossy()
                .eq_ignore_ascii_case(&b.to_string_lossy())
        };
        let mut vars: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(name, _)| !changes.iter().any(|(changed, _)| same(name, changed)))
            .collect();
        for (name, value) in &changes {
            if let Some(value) = value {
                vars.push((name.to_os_string(), value.to_os_string()));
            }
        }
        let mut block = Vec::new();
        for (name, value) in &vars {
            block.extend(name.encode_wide());
            block.push(u16::from(b'='));
            block.extend(value.encode_wide());
            block.push(0);
        }
        if vars.is_empty() {
            block.push(0);
        }
        block.push(0);
        block
    }

    pub(crate) fn spawn(cmd: &Command) -> io::Result<Piped> {
        let (child_stdin, parent_stdin) = pipe()?;
        let (parent_stdout, child_stdout) = pipe()?;
        let (parent_stderr, child_stderr) = pipe()?;
        for handle in [&child_stdin, &child_stdout, &child_stderr] {
            inheritable(handle)?;
        }
        let handles: [Handle; 3] = [
            child_stdin.as_raw_handle() as Handle,
            child_stdout.as_raw_handle() as Handle,
            child_stderr.as_raw_handle() as Handle,
        ];

        let mut line = command_line(cmd.get_program(), cmd.get_args());
        let block = environment_block(cmd.get_envs());
        let directory: Option<Vec<u16>> = cmd.get_current_dir().map(|dir| {
            dir.as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        });

        // SAFETY: plain Win32 calls. Every pointer is to a live local: the
        // attribute list buffer and `handles` outlive CreateProcessW, the
        // strings are NUL-terminated, and the structs are zeroed where
        // Windows wants zero.
        let created = unsafe {
            let mut size = 0usize;
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size);
            let mut list = vec![0u8; size.max(1)];
            let list_ptr = list.as_mut_ptr() as *mut c_void;
            if InitializeProcThreadAttributeList(list_ptr, 1, 0, &mut size) == 0 {
                Err(io::Error::last_os_error())
            } else {
                let updated = UpdateProcThreadAttribute(
                    list_ptr,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                    handles.as_ptr() as *const c_void,
                    std::mem::size_of_val(&handles),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
                let started = if updated == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    let mut info: StartupInfoEx = std::mem::zeroed();
                    info.startup.cb = std::mem::size_of::<StartupInfoEx>() as u32;
                    info.startup.flags = STARTF_USESTDHANDLES;
                    info.startup.std_input = handles[0];
                    info.startup.std_output = handles[1];
                    info.startup.std_error = handles[2];
                    info.attribute_list = list_ptr;
                    let mut process: ProcessInformation = std::mem::zeroed();
                    let ok = CreateProcessW(
                        std::ptr::null(),
                        line.as_mut_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        1,
                        CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
                        block.as_ptr() as *const c_void,
                        directory
                            .as_ref()
                            .map_or(std::ptr::null(), |dir| dir.as_ptr()),
                        &info.startup,
                        &mut process,
                    );
                    if ok == 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        CloseHandle(process.thread);
                        Ok(Process {
                            handle: OwnedHandle::from_raw_handle(process.process as _),
                        })
                    }
                };
                DeleteProcThreadAttributeList(list_ptr);
                started
            }
        };
        // The child has its own copies now. Closing these ends is what lets
        // a reader see end-of-file when the child exits.
        drop((child_stdin, child_stdout, child_stderr));
        let process = created?;
        Ok(Piped {
            proc: Proc::Raw(process),
            stdin: Box::new(File::from(parent_stdin)),
            stdout: Box::new(File::from(parent_stdout)),
            stderr: Box::new(File::from(parent_stderr)),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn line(program: &str, args: &[&str]) -> String {
            let units = command_line(OsStr::new(program), args.iter().map(OsStr::new));
            assert_eq!(units.last(), Some(&0));
            String::from_utf16(&units[..units.len() - 1]).unwrap()
        }

        #[test]
        fn quotes_only_what_the_parser_needs() {
            assert_eq!(line("a.exe", &["b", "c d"]), r#""a.exe" b "c d""#);
            assert_eq!(line("a.exe", &[""]), r#""a.exe" """#);
            assert_eq!(
                line(r"C:\x y\a.exe", &["-o", r"C:\out dir\b.exe"]),
                r#""C:\x y\a.exe" -o "C:\out dir\b.exe""#
            );
            // A quote gets a backslash; backslashes double only before a quote.
            assert_eq!(line("a", &[r#"say "hi""#]), r#""a" "say \"hi\"""#);
            assert_eq!(line("a", &[r"C:\dir\"]), r#""a" C:\dir\"#);
            assert_eq!(line("a", &[r"C:\my dir\"]), r#""a" "C:\my dir\\""#);
            assert_eq!(line("a", &[r#"x\"y"#]), r#""a" x\\\"y"#);
        }

        #[test]
        fn batch_files_go_through_command() {
            assert!(is_batch_file(OsStr::new("lust.cmd")));
            assert!(is_batch_file(OsStr::new(r"C:\tools\run.BAT")));
            assert!(!is_batch_file(OsStr::new("lust.exe")));
            assert!(!is_batch_file(OsStr::new("lust")));
        }

        #[test]
        fn the_block_applies_changes_without_case() {
            let home = std::env::vars_os().next();
            let set = (OsStr::new("CRAB_CAGE_BLOCK_TEST"), Some(OsStr::new("v w")));
            let removed = (OsStr::new("path"), None);
            let block = environment_block([set, removed].into_iter());
            assert!(block.ends_with(&[0, 0]));
            let text = String::from_utf16_lossy(&block);
            let entries: Vec<&str> = text.split('\0').filter(|e| !e.is_empty()).collect();
            assert!(entries.contains(&"CRAB_CAGE_BLOCK_TEST=v w"));
            assert!(!entries
                .iter()
                .any(|entry| entry.to_ascii_uppercase().starts_with("PATH=")));
            if let Some((name, value)) = home {
                if !name.to_string_lossy().eq_ignore_ascii_case("path") {
                    let entry = format!("{}={}", name.to_string_lossy(), value.to_string_lossy());
                    assert!(entries.contains(&entry.as_str()));
                }
            }
        }
    }
}
