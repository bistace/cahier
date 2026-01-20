use anyhow::{Context, Result};
use crossterm::terminal;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

#[cfg(unix)]
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
#[cfg(unix)]
use nix::unistd::Pid;

use crate::common;

/// RAII guard that ensures raw mode is disabled when dropped
struct RawModeGuard {
    active: bool,
}

impl RawModeGuard {
    fn new() -> Result<Self> {
        match terminal::enable_raw_mode() {
            Ok(_) => Ok(RawModeGuard { active: true }),
            Err(e) => {
                // Log warning but don't fail. This allows tests to run in non-TTY environments.
                eprintln!("Warning: Failed to enable raw mode: {}", e);
                Ok(RawModeGuard { active: false })
            }
        }
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = terminal::disable_raw_mode();
        }
    }
}

/// Helper for non-blocking stdin
struct NonBlockingStdinGuard {
    #[cfg(unix)]
    fd: i32,
    #[cfg(unix)]
    orig_flags: i32,
}

impl NonBlockingStdinGuard {
    fn new() -> Result<Self> {
        #[cfg(unix)]
        {
            let fd = std::io::stdin().as_raw_fd();
            // SAFETY: fd is the file descriptor for stdin, which is guaranteed to be valid here.
            let orig_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if orig_flags < 0 {
                return Err(anyhow::anyhow!("Failed to get stdin flags"));
            }

            // SAFETY: fd is valid and we are setting the O_NONBLOCK flag to enable non-blocking reads.
            let res = unsafe { libc::fcntl(fd, libc::F_SETFL, orig_flags | libc::O_NONBLOCK) };
            if res < 0 {
                return Err(anyhow::anyhow!("Failed to set stdin to non-blocking"));
            }

            Ok(Self { fd, orig_flags })
        }
        #[cfg(not(unix))]
        {
            // On non-Unix, we might not easily set non-blocking stdin without external crates or complex logic.
            // For now, we proceed without it, which might mean the input thread blocks on read.
            Ok(Self {})
        }
    }
}

impl Drop for NonBlockingStdinGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            // SAFETY: self.fd is a valid file descriptor (stdin) and self.orig_flags are the original flags.
            // We are restoring the original flags.
            unsafe {
                if libc::fcntl(self.fd, libc::F_SETFL, self.orig_flags) < 0 {
                    eprintln!("Failed to restore stdin flags");
                }
            }
        }
    }
}

/// Guard that ensures the temporary environment file is deleted
struct EnvDumpGuard(PathBuf);

impl Drop for EnvDumpGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn create_secure_temp_file() -> Result<PathBuf> {
    // Resolve to absolute path to ensure it works even if the child process changes directory
    let temp_dir = common::temp_dir();

    if !temp_dir.exists() {
        fs::create_dir_all(&temp_dir).context("Failed to create temp directory")?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(&temp_dir)?;
        let mut perms = metadata.permissions();
        if perms.mode() & 0o777 != 0o700 {
            perms.set_mode(0o700);
            fs::set_permissions(&temp_dir, perms).context("Failed to set temp dir permissions")?;
        }
    }

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%f");
    let filename = format!("cahier_env_{}", timestamp);
    let filepath = temp_dir.join(filename);

    // Create the file with restrictive permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&filepath)
            .context("Failed to create secure temp file")?;
    }
    #[cfg(not(unix))]
    {
        // On non-Unix, standard creation. Access control depends on ACLs which we don't explicitly manage here yet.
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&filepath)
            .context("Failed to create temp file")?;
    }

    Ok(filepath)
}

fn quote_for_trap(path: &str) -> String {
    // We want to produce: umask 077; (env -0 || printenv -0) > "PATH"
    // PATH needs " and \ escaped.
    let escaped_path = path.replace('\\', "\\\\").replace('"', "\\\"");

    // Try `env -0`, fallback to `printenv -0`, fallback to nothing (or true) if both fail to avoid crash,
    // though we won't get env vars.
    // We use `2>/dev/null` to silence errors if tools are missing.
    let inner_cmd = format!(
        "umask 077; (env -0 2>/dev/null || printenv -0 2>/dev/null || true) > \"{}\"",
        escaped_path
    );

    // Now quote for the single-quoted trap argument
    // replace ' with '\''
    let trap_arg = inner_cmd.replace('\'', "'\\''");
    format!("'{}'", trap_arg)
}

struct OutputHandler {
    captured_output: Vec<u8>,
    output_file: Option<File>,
    output_filename: Option<String>,
    max_output_size: usize,
    capture_output: bool,
    suppress_output: bool,
    last_flush: std::time::Instant,
}

impl OutputHandler {
    fn new(max_output_size: usize, capture_output: bool, suppress_output: bool) -> Self {
        Self {
            captured_output: Vec::new(),
            output_file: None,
            output_filename: None,
            max_output_size,
            capture_output,
            suppress_output,
            last_flush: std::time::Instant::now(),
        }
    }

    fn handle_data(&mut self, data: &[u8]) -> std::io::Result<()> {
        if self.capture_output
            && self.captured_output.len() + data.len() > self.max_output_size
            && self.output_file.is_none()
        {
            let output_dir = common::output_dir();
            if !output_dir.exists() {
                let _ = std::fs::create_dir_all(&output_dir);

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(metadata) = std::fs::metadata(&output_dir) {
                        let mut perms = metadata.permissions();
                        if perms.mode() & 0o777 != 0o700 {
                            perms.set_mode(0o700);
                            let _ = std::fs::set_permissions(&output_dir, perms);
                        }
                    }
                }
            }

            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            let filename = format!("output_{}.txt", timestamp);
            let filepath = output_dir.join(&filename);

            #[cfg(unix)]
            let file_res = {
                use std::os::unix::fs::OpenOptionsExt;
                fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&filepath)
            };
            #[cfg(not(unix))]
            let file_res = File::create(&filepath);

            if let Ok(mut file) = file_res {
                let _ = file.write_all(&self.captured_output);
                self.output_filename = Some(filepath.to_string_lossy().to_string());
                self.output_file = Some(file);
            }

            self.captured_output.clear();
            self.captured_output.shrink_to_fit();

            if let Some(ref name) = self.output_filename {
                self.captured_output.extend_from_slice(
                    format!("[Output too large, redirected to {}]\n", name).as_bytes(),
                );
            }
        }

        if self.capture_output {
            if let Some(ref mut file) = self.output_file {
                file.write_all(data)?;
            } else {
                self.captured_output.extend_from_slice(data);
            }
        }

        if !self.suppress_output {
            // Write to stdout with retry on EAGAIN/WouldBlock (common with fullscreen apps like top)
            let mut written = 0;
            while written < data.len() {
                match io::stdout().write(&data[written..]) {
                    Ok(0) => break, // Can't write more
                    Ok(n) => written += n,
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(1));
                    }
                    Err(e) => return Err(e),
                }
            }
            // Flush periodically (e.g., every 50ms) or on newline to avoid excessive syscalls
            // while keeping it interactive.
            if data.contains(&b'\n') || self.last_flush.elapsed().as_millis() > 50 {
                loop {
                    match io::stdout().flush() {
                        Ok(()) => break,
                        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(std::time::Duration::from_millis(1));
                        }
                        Err(e) => return Err(e),
                    }
                }
                self.last_flush = std::time::Instant::now();
            }
        }
        Ok(())
    }

    fn finalize(self) -> (String, Option<String>) {
        (
            String::from_utf8_lossy(&self.captured_output).to_string(),
            self.output_filename,
        )
    }
}

pub struct Job {
    pub id: usize,
    pub command: String,
    pub child: Box<dyn Child + Send + Sync>,
    pub master: Box<dyn MasterPty + Send>,
    pub writer: Option<Box<dyn Write + Send>>,
    pub env_dump_path: PathBuf,
}

pub enum ExecutionResult {
    Completed {
        output: String,
        exit_code: Option<i32>,
        output_file: Option<String>,
    },
    Suspended(Job),
}

struct MonitorConfig {
    command: String,
    env_dump_path: PathBuf,
    max_output_size: usize,
    capture_output: bool,
    existing_writer: Option<Box<dyn Write + Send>>,
    suppress_output: bool,
}

// --- Refactored Helper Functions ---

/// Spawns a background thread to forward stdin to the PTY master
fn spawn_input_forwarding_thread(
    pty_writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    finished: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        // Set stdin to non-blocking mode
        let _nonblocking_guard = match NonBlockingStdinGuard::new() {
            Ok(g) => g,
            Err(e) => {
                eprintln!("Failed to set stdin non-blocking: {}", e);
                return;
            }
        };

        let mut stdin = io::stdin();
        let mut buf = [0u8; 1024];
        loop {
            if finished.load(Ordering::Relaxed) {
                break;
            }

            match stdin.read(&mut buf) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    // Write to the PTY master
                    // Handle mutex poisoning by ignoring the error and breaking, or unwrapping if we prefer panicking on poison
                    match pty_writer.lock() {
                        Ok(mut writer_opt) => {
                            if let Some(writer) = writer_opt.as_mut() {
                                if writer.write_all(&buf[..n]).is_err() {
                                    break; // PTY closed
                                }
                                let _ = writer.flush();
                            } else {
                                break; // No writer available
                            }
                        }
                        Err(_) => break, // Lock failed
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // No data available, sleep briefly to avoid busy loop
                    thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(_) => break, // Read error
            }
        }
    })
}

/// Checks the process status without blocking
fn check_process_status(
    child: &mut Box<dyn Child + Send + Sync>,
    _suspended: &mut bool,
) -> Option<i32> {
    #[cfg(unix)]
    {
        if *_suspended {
            return None;
        }
        if let Some(pid_val) = child.process_id() {
            let pid = Pid::from_raw(pid_val as i32);
            match waitpid(pid, Some(WaitPidFlag::WNOHANG | WaitPidFlag::WUNTRACED)) {
                Ok(WaitStatus::Stopped(_, _)) => {
                    *_suspended = true;
                    None
                }
                Ok(WaitStatus::Exited(_, code)) => Some(code),
                Ok(WaitStatus::Signaled(_, sig, _)) => Some(128 + (sig as i32)),
                Err(nix::errno::Errno::ECHILD) => Some(1), // Process likely gone
                _ => None,
            }
        } else {
            None
        }
    }

    #[cfg(not(unix))]
    {
        if let Ok(Some(status)) = child.try_wait() {
            if status.success() {
                Some(0)
            } else {
                Some(1)
            }
        } else {
            None
        }
    }
}

/// Polls the PTY master for output data
fn poll_pty_output(
    _master_fd: i32,
    reader: &mut Box<dyn Read + Send>,
    output_handler: &mut OutputHandler,
    buf: &mut [u8],
) -> Result<bool> {
    // returns true if EOF
    #[cfg(unix)]
    {
        // SAFETY: master_fd is valid and kept open by master, so borrowing it is safe.
        let borrowed_fd = unsafe { std::os::unix::io::BorrowedFd::borrow_raw(_master_fd) };
        let mut fds = [nix::poll::PollFd::new(
            borrowed_fd,
            nix::poll::PollFlags::POLLIN,
        )];
        match nix::poll::poll(&mut fds, nix::poll::PollTimeout::from(50u16)) {
            Ok(_) => {
                if let Some(revents) = fds[0].revents() {
                    if revents.contains(nix::poll::PollFlags::POLLIN)
                        || revents.contains(nix::poll::PollFlags::POLLHUP)
                        || revents.contains(nix::poll::PollFlags::POLLERR)
                    {
                        match reader.read(buf) {
                            Ok(0) => return Ok(true), // EOF
                            Ok(n) => {
                                output_handler.handle_data(&buf[..n])?;
                            }
                            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                            Err(e) => return Err(e.into()),
                        }
                    }
                }
            }
            Err(nix::errno::Errno::EINTR) => {}
            Err(e) => return Err(e.into()),
        }
    }

    #[cfg(not(unix))]
    {
        // Blocking read fallback for non-unix
        match reader.read(buf) {
            Ok(0) => return Ok(true),
            Ok(n) => {
                output_handler.handle_data(&buf[..n])?;
            }
            Err(e) => return Err(e.into()),
        }
    }

    Ok(false)
}

/// Parses the environment dump file and updates current_env
fn process_env_dump(path: &PathBuf, current_env: &Arc<Mutex<HashMap<String, String>>>) {
    if let Ok(env_data) = fs::read(path) {
        // Parse null-terminated environment variables into a temporary HashMap
        let mut new_env = HashMap::new();
        for entry in env_data.split(|&b| b == 0) {
            if entry.is_empty() {
                continue;
            }
            if let Ok(s) = std::str::from_utf8(entry) {
                if let Some(pos) = s.find('=') {
                    let key = s[..pos].to_string();
                    let value = s[pos + 1..].to_string();
                    new_env.insert(key, value);
                }
            }
        }

        // Only update current_env if we successfully parsed at least some variables
        if !new_env.is_empty() {
            // Check for PWD change and sync if needed
            if let Some(pwd) = new_env.get("PWD") {
                let current_pwd = std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();

                if pwd != &current_pwd {
                    if let Err(e) = std::env::set_current_dir(pwd) {
                        eprintln!("Failed to sync PWD from shell: {}", e);
                    }
                }
            }

            if let Ok(mut env) = current_env.lock() {
                *env = new_env;
            } else {
                eprintln!("Failed to lock current_env for update");
            }
        }
    }
}

/// Runs the main monitoring loop checking for process status and PTY output
fn run_monitoring_loop(
    child: &mut Box<dyn Child + Send + Sync>,
    master_fd: i32,
    reader: &mut Box<dyn Read + Send>,
    output_handler: &mut OutputHandler,
) -> Result<(Option<i32>, bool)> {
    let mut buf = [0u8; 1024];
    let mut exit_code = None;
    let mut suspended = false;

    loop {
        // 1. Check process status (non-blocking)
        if let Some(code) = check_process_status(child, &mut suspended) {
            exit_code = Some(code);
        }

        if suspended {
            break;
        }

        // 2. Poll for data with timeout
        let eof = poll_pty_output(master_fd, reader, output_handler, &mut buf)?;

        if eof {
            break;
        }
    }

    // Ensure exit code is set if we broke out due to EOF but waitpid didn't catch it yet
    if exit_code.is_none() && !suspended {
        #[cfg(unix)]
        {
            if let Some(pid_val) = child.process_id() {
                let pid = Pid::from_raw(pid_val as i32);
                // Wait blocking now since we are done reading
                match waitpid(pid, None) {
                    Ok(WaitStatus::Exited(_, code)) => {
                        exit_code = Some(code);
                    }
                    Ok(WaitStatus::Signaled(_, sig, _)) => {
                        exit_code = Some(128 + (sig as i32));
                    }
                    _ => {}
                }
            }
        }
        #[cfg(not(unix))]
        {
            if let Ok(status) = child.wait() {
                exit_code = if status.success() { Some(0) } else { Some(1) };
            }
        }
    }

    Ok((exit_code, suspended))
}

/// Monitors the execution of a PTY process (handles I/O and waiting)
fn monitor_execution(
    mut child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    pty_writer: &Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    current_env: &Arc<Mutex<HashMap<String, String>>>,
    config: MonitorConfig,
) -> Result<ExecutionResult> {
    // Register the writer for Ctrl+C handling
    let writer = if let Some(w) = config.existing_writer {
        w
    } else {
        master
            .take_writer()
            .context("Failed to take writer from master PTY")?
    };

    {
        let mut writer_opt = pty_writer
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to lock pty_writer"))?;
        *writer_opt = Some(writer);
    }

    let mut reader = master
        .try_clone_reader()
        .context("Failed to clone reader from master PTY")?;

    // Enable raw mode to forward all keystrokes (including Ctrl+X, etc.) to the child
    let _raw_mode_guard = RawModeGuard::new()?;

    // Flag to signal input thread to stop
    let finished = Arc::new(AtomicBool::new(false));

    // Spawn a thread to forward stdin to the PTY master
    let input_thread = spawn_input_forwarding_thread(Arc::clone(pty_writer), Arc::clone(&finished));

    let mut output_handler = OutputHandler::new(
        config.max_output_size,
        config.capture_output,
        config.suppress_output,
    );

    // Ensure env dump file is cleaned up
    let _env_dump_guard = EnvDumpGuard(config.env_dump_path.clone());

    #[cfg(unix)]
    // We assume master_fd is valid on Unix.
    let master_fd = master
        .as_raw_fd()
        .ok_or_else(|| anyhow::anyhow!("Failed to get raw fd from PTY master"))?;
    #[cfg(not(unix))]
    let master_fd = 0; // Dummy for non-unix

    let (exit_code, suspended) =
        run_monitoring_loop(&mut child, master_fd, &mut reader, &mut output_handler)?;

    // Signal input thread to stop
    finished.store(true, Ordering::Relaxed);

    // The input thread will stop naturally when it sees the flag or writer is cleared.
    let _ = input_thread.join();

    // Ensure we clean up pty_writer and retrieve it if suspended
    let retrieved_writer = {
        let mut writer_opt = pty_writer
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to lock pty_writer"))?;
        writer_opt.take()
    };

    if suspended {
        return Ok(ExecutionResult::Suspended(Job {
            id: 0, // ID assigned by caller
            command: config.command,
            child,
            master,
            writer: retrieved_writer,
            env_dump_path: config.env_dump_path,
        }));
    }

    // Read and parse the environment dump
    process_env_dump(&config.env_dump_path, current_env);

    let (output, output_file) = output_handler.finalize();

    Ok(ExecutionResult::Completed {
        output,
        exit_code,
        output_file,
    })
}

/// Executes a command in a PTY environment, capturing output and handling signals.
pub fn execute_in_pty(
    command: &str,
    max_output_size: usize,
    pty_writer: &Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    current_env: &Arc<Mutex<HashMap<String, String>>>,
    capture_output: bool,
    suppress_output: bool,
) -> Result<ExecutionResult> {
    let pty_system = native_pty_system();

    // Get terminal size for the PTY to match current term
    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("Failed to open PTY")?;

    // Use the user's shell or default to sh
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
    let mut cmd = CommandBuilder::new(shell);

    // Generate unique temporary file path for environment dump
    // We pre-create it to ensure we have permissions right (security hardening)
    let env_dump_path = create_secure_temp_file()?;

    // Wrap command with trap to capture environment on exit
    // Set umask 077 to ensure the temporary file is only readable by the owner
    let wrapped_command = format!(
        "trap {} EXIT; {}",
        quote_for_trap(&env_dump_path.to_string_lossy()),
        command
    );
    cmd.args(["-c", &wrapped_command]);

    // Clear and set environment from current_env
    cmd.env_clear();
    {
        let env = current_env
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to lock current_env"))?;
        for (key, value) in env.iter() {
            cmd.env(key, value);
        }
    }

    // Set current directory
    if let Ok(path) = std::env::current_dir() {
        cmd.cwd(path);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .context("Failed to spawn command in PTY")?;

    // Drop slave to close the write-end of the pipe in this process
    drop(pair.slave);

    monitor_execution(
        child,
        pair.master,
        pty_writer,
        current_env,
        MonitorConfig {
            command: command.to_string(),
            env_dump_path,
            max_output_size,
            capture_output,
            existing_writer: None,
            suppress_output,
        },
    )
}

pub fn resume_job(
    job: Job,
    max_output_size: usize,
    pty_writer: &Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    current_env: &Arc<Mutex<HashMap<String, String>>>,
) -> Result<ExecutionResult> {
    // Resize PTY to match current terminal
    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    let _ = job.master.resize(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    });

    // Send SIGCONT
    #[cfg(unix)]
    {
        if let Some(pid_val) = job.child.process_id() {
            let pid = Pid::from_raw(pid_val as i32);
            let pgid = Pid::from_raw(-(pid_val as i32));

            // Send SIGCONT to the process group
            let _ = nix::sys::signal::kill(pgid, nix::sys::signal::Signal::SIGCONT);

            // Fallback: Send SIGCONT to the process directly if it's not a group leader or something went wrong
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGCONT);

            // Send SIGWINCH to force repaint
            let _ = nix::sys::signal::kill(pgid, nix::sys::signal::Signal::SIGWINCH);
        }
    }

    monitor_execution(
        job.child,
        job.master,
        pty_writer,
        current_env,
        MonitorConfig {
            command: job.command,
            env_dump_path: job.env_dump_path,
            max_output_size,
            capture_output: false, // Don't capture output on resume
            existing_writer: job.writer,
            suppress_output: false,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_in_pty() {
        // simple echo
        let pty_writer = Arc::new(Mutex::new(None));
        let env = Arc::new(Mutex::new(std::env::vars().collect()));
        // We must ensure CAHIER_DIR exists for tests if we use create_secure_temp_file
        let _ = std::fs::create_dir_all(crate::common::temp_dir());

        let result = execute_in_pty("echo 'hello world'", 1024, &pty_writer, &env, true, true)
            .expect("failed to execute");

        match result {
            ExecutionResult::Completed {
                output,
                exit_code,
                output_file,
            } => {
                assert!(output.contains("hello world"));
                assert_eq!(exit_code, Some(0));
                assert!(output_file.is_none());
            }
            _ => panic!("Expected completion"),
        }
    }

    #[test]
    fn test_execute_failure() {
        let pty_writer = Arc::new(Mutex::new(None));
        let env = Arc::new(Mutex::new(std::env::vars().collect()));
        let _ = std::fs::create_dir_all(crate::common::temp_dir());
        let result = execute_in_pty(
            "nonexistent_command_123",
            1024,
            &pty_writer,
            &env,
            true,
            true,
        )
        .expect("failed to execute");

        match result {
            ExecutionResult::Completed { exit_code, .. } => {
                assert_ne!(exit_code, Some(0));
            }
            _ => panic!("Expected completion"),
        }
    }

    #[test]
    fn test_quote_for_trap() {
        let path = "/tmp/test path/env";
        let quoted = quote_for_trap(path);
        // Should produce: 'umask 077; (env -0 2>/dev/null || printenv -0 2>/dev/null || true) > "/tmp/test path/env"'
        assert_eq!(quoted, "'umask 077; (env -0 2>/dev/null || printenv -0 2>/dev/null || true) > \"/tmp/test path/env\"'");

        let path_with_quote = "/tmp/test\"path/env";
        let quoted_quote = quote_for_trap(path_with_quote);
        // Should escape internal quote: " -> \"
        assert_eq!(quoted_quote, "'umask 077; (env -0 2>/dev/null || printenv -0 2>/dev/null || true) > \"/tmp/test\\\"path/env\"'");

        let path_with_single_quote = "/tmp/test'path";
        let quoted_single = quote_for_trap(path_with_single_quote);
        // Single quote in single quoted string needs ' -> '\''
        assert_eq!(quoted_single, "'umask 077; (env -0 2>/dev/null || printenv -0 2>/dev/null || true) > \"/tmp/test'\\''path\"'");
    }
}
