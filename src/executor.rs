use anyhow::Result;
use crossterm::terminal;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize, Child};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::os::unix::io::AsRawFd;

#[cfg(unix)]
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
#[cfg(unix)]
use nix::unistd::Pid;

use crate::common::OUTPUT_DIR;

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
                 // We could verify if e is "No such device or address" but for now simply catching all errors is fine for robustness.
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
    fd: i32,
    orig_flags: i32,
}

impl NonBlockingStdinGuard {
    fn new() -> Result<Self> {
        let fd = std::io::stdin().as_raw_fd();
        let orig_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if orig_flags < 0 {
            return Err(anyhow::anyhow!("Failed to get stdin flags"));
        }
        
        let res = unsafe { libc::fcntl(fd, libc::F_SETFL, orig_flags | libc::O_NONBLOCK) };
        if res < 0 {
            return Err(anyhow::anyhow!("Failed to set stdin to non-blocking"));
        }

        Ok(Self { fd, orig_flags })
    }
}

impl Drop for NonBlockingStdinGuard {
    fn drop(&mut self) {
        unsafe {
            if libc::fcntl(self.fd, libc::F_SETFL, self.orig_flags) < 0 {
                eprintln!("Failed to restore stdin flags");
            }
        }
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

/// Monitors the execution of a PTY process (handles I/O and waiting)
fn monitor_execution(
    command: String,
    mut child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    env_dump_path: PathBuf,
    max_output_size: usize,
    pty_writer: &Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    current_env: &Arc<Mutex<HashMap<String, String>>>,
    capture_output: bool,
    existing_writer: Option<Box<dyn Write + Send>>,
) -> Result<ExecutionResult> {
    // Register the writer for Ctrl+C handling
    let writer = if let Some(w) = existing_writer {
        w
    } else {
        master.take_writer()?
    };

    {
        let mut writer_opt = pty_writer.lock().unwrap();
        *writer_opt = Some(writer);
    }

    let mut reader = master.try_clone_reader()?;

    // Enable raw mode to forward all keystrokes (including Ctrl+X, etc.) to the child
    let _raw_mode_guard = RawModeGuard::new()?;

    // Flag to signal input thread to stop
    let finished = Arc::new(AtomicBool::new(false));
    let finished_clone = finished.clone();

    // Spawn a thread to forward stdin to the PTY master
    let pty_writer_clone = Arc::clone(pty_writer);
    let input_thread = thread::spawn(move || {
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
            if finished_clone.load(Ordering::Relaxed) {
                break;
            }

            match stdin.read(&mut buf) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    // Write to the PTY master
                    if let Ok(mut writer_opt) = pty_writer_clone.lock() {
                        if let Some(writer) = writer_opt.as_mut() {
                            if writer.write_all(&buf[..n]).is_err() {
                                break; // PTY closed
                            }
                            let _ = writer.flush();
                        } else {
                            break; // No writer available
                        }
                    } else {
                        break; // Lock failed
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // No data available, sleep briefly to avoid busy loop
                    thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(_) => break, // Read error
            }
        }
    });

    let mut buf = [0u8; 1024];
    let mut captured_output = Vec::new();
    let mut output_file: Option<File> = None;
    let mut output_filename: Option<String> = None;

    let mut exit_code = None;
    let mut suspended = false;

    #[cfg(unix)]
    // We assume master_fd is valid on Unix. If it's somehow not (which shouldn't happen with MasterPty on unix), we panic.
    let master_fd = master.as_raw_fd().expect("Failed to get raw fd from PTY master");

    // Loop to check status and read
    loop {
        // 1. Check process status (non-blocking)
        #[cfg(unix)]
        if !suspended && exit_code.is_none() {
            if let Some(pid_val) = child.process_id() {
                 let pid = Pid::from_raw(pid_val as i32);
                 match waitpid(pid, Some(WaitPidFlag::WNOHANG | WaitPidFlag::WUNTRACED)) {
                     Ok(WaitStatus::Stopped(_, _)) => {
                         suspended = true;
                         break;
                     }
                     Ok(WaitStatus::Exited(_, code)) => {
                         exit_code = Some(code);
                     }
                     Ok(WaitStatus::Signaled(_, sig, _)) => {
                         exit_code = Some(128 + (sig as i32));
                     }
                     Err(nix::errno::Errno::ECHILD) => {
                         // Process likely gone, maybe we missed the signal or it was reaped elsewhere?
                         if exit_code.is_none() { exit_code = Some(1); }
                     }
                     _ => {}
                 }
            }
        }
        
        #[cfg(not(unix))]
        if exit_code.is_none() {
             // Fallback for non-unix: just check if process is running? 
             // child.try_wait() returns Ok(Some(status)) if exited.
             if let Ok(Some(status)) = child.try_wait() {
                 exit_code = if status.success() { Some(0) } else { Some(1) };
             }
        }

        // 2. Poll for data with timeout
        #[cfg(unix)]
        {
            // Safety: master_fd is valid and kept open by master
            let borrowed_fd = unsafe { std::os::unix::io::BorrowedFd::borrow_raw(master_fd) };
            let mut fds = [nix::poll::PollFd::new(borrowed_fd, nix::poll::PollFlags::POLLIN)];
            match nix::poll::poll(&mut fds, nix::poll::PollTimeout::from(50u16)) {
                Ok(_) => {
                     if let Some(revents) = fds[0].revents() {
                         if revents.contains(nix::poll::PollFlags::POLLIN) || revents.contains(nix::poll::PollFlags::POLLHUP) || revents.contains(nix::poll::PollFlags::POLLERR) {
                             // Try reading
                             match reader.read(&mut buf) {
                                 Ok(0) => break, // EOF
                                 Ok(n) => {
                                     let data = &buf[..n];

                                     // Process output (capture/print)
                                     if capture_output && captured_output.len() + n > max_output_size && output_file.is_none() {
                                        let output_dir = PathBuf::from(OUTPUT_DIR);
                                        let _ = std::fs::create_dir_all(&output_dir);
                                        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
                                        let filename = format!("output_{}.txt", timestamp);
                                        let filepath = output_dir.join(&filename);

                                        if let Ok(mut file) = File::create(&filepath) {
                                             let _ = file.write_all(&captured_output);
                                             output_filename = Some(format!("{}/{}", OUTPUT_DIR, filename));
                                             output_file = Some(file);
                                        }
                                        captured_output.clear();
                                        if let Some(ref name) = output_filename {
                                            captured_output.extend_from_slice(format!("[Output too large, redirected to {}]\n", name).as_bytes());
                                        }
                                     }

                                     if capture_output {
                                         if let Some(ref mut file) = output_file {
                                             let _ = file.write_all(data);
                                         } else {
                                             captured_output.extend_from_slice(data);
                                         }
                                     }

                                     let _ = io::stdout().write_all(data);
                                     let _ = io::stdout().flush();
                                 }
                                 Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                                     // Continue
                                 }
                                 Err(_) => break,
                             }
                         }
                     }
                }
                Err(nix::errno::Errno::EINTR) => continue,
                Err(_) => break,
            }
        }

        #[cfg(not(unix))]
        {
            // Blocking read fallback for non-unix
            match reader.read(&mut buf) {
                 Ok(0) => break,
                 Ok(n) => {
                      let data = &buf[..n];
                      // ... duplicate logic for capture/print ... 
                       if capture_output && captured_output.len() + n > max_output_size && output_file.is_none() {
                            let output_dir = PathBuf::from(OUTPUT_DIR);
                            let _ = std::fs::create_dir_all(&output_dir);
                            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
                            let filename = format!("output_{}.txt", timestamp);
                            let filepath = output_dir.join(&filename);

                            if let Ok(mut file) = File::create(&filepath) {
                                 let _ = file.write_all(&captured_output);
                                 output_filename = Some(format!("{}/{}", OUTPUT_DIR, filename));
                                 output_file = Some(file);
                            }
                            captured_output.clear();
                            if let Some(ref name) = output_filename {
                                captured_output.extend_from_slice(format!("[Output too large, redirected to {}]\n", name).as_bytes());
                            }
                       }
                       if capture_output {
                            if let Some(ref mut file) = output_file {
                                let _ = file.write_all(data);
                            } else {
                                captured_output.extend_from_slice(data);
                            }
                       }
                       let _ = io::stdout().write_all(data);
                       let _ = io::stdout().flush();
                 }
                 Err(_) => break,
            }
            if exit_code.is_some() { break; }
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
    
    // Signal input thread to stop
    finished.store(true, Ordering::Relaxed);

    // The input thread will stop naturally when it sees the flag or writer is cleared.
    let _ = input_thread.join();

    // Ensure we clean up pty_writer and retrieve it if suspended
    let retrieved_writer = {
        let mut writer_opt = pty_writer.lock().unwrap();
        writer_opt.take()
    };

    if suspended {
        return Ok(ExecutionResult::Suspended(Job {
            id: 0, // ID assigned by caller
            command,
            child,
            master,
            writer: retrieved_writer,
            env_dump_path,
        }));
    }

    // Read and parse the environment dump
    if let Ok(env_data) = fs::read(&env_dump_path) {
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
            let mut env = current_env.lock().unwrap();
            *env = new_env;
        }

        // Clean up the temp file
        let _ = fs::remove_file(&env_dump_path);
    }

    Ok(ExecutionResult::Completed {
        output: String::from_utf8_lossy(&captured_output).to_string(),
        exit_code,
        output_file: output_filename,
    })
}

/// Executes a command in a PTY environment, capturing output and handling signals.
pub fn execute_in_pty(
    command: &str,
    max_output_size: usize,
    pty_writer: &Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    current_env: &Arc<Mutex<HashMap<String, String>>>,
    capture_output: bool,
) -> Result<ExecutionResult> {
    let pty_system = native_pty_system();

    // Get terminal size for the PTY to match current term
    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    // Use the user's shell or default to sh
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
    let mut cmd = CommandBuilder::new(shell);

    // Generate unique temporary file path for environment dump
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%f");
    let env_dump_path = std::env::temp_dir().join(format!("cahier_env_{}", timestamp));

    // Wrap command with trap to capture environment on exit
    let wrapped_command = format!(
        "trap 'env -0 > \"{}\"' EXIT; {}",
        env_dump_path.display(),
        command
    );
    cmd.args(["-c", &wrapped_command]);

    // Clear and set environment from current_env
    cmd.env_clear();
    {
        let env = current_env.lock().unwrap();
        for (key, value) in env.iter() {
            cmd.env(key, value);
        }
    }

    // Set current directory
    if let Ok(path) = std::env::current_dir() {
        cmd.cwd(path);
    }

    let child = pair.slave.spawn_command(cmd)?;

    // Drop slave to close the write-end of the pipe in this process
    drop(pair.slave);

    monitor_execution(
        command.to_string(),
        child,
        pair.master,
        env_dump_path,
        max_output_size,
        pty_writer,
        current_env,
        capture_output,
        None,
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
        job.command,
        job.child,
        job.master,
        job.env_dump_path,
        max_output_size,
        pty_writer,
        current_env,
        false, // Don't capture output on resume
        job.writer,
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
        let result =
            execute_in_pty("echo 'hello world'", 1024, &pty_writer, &env, true)
                .expect("failed to execute");
        
        match result {
            ExecutionResult::Completed { output, exit_code, output_file } => {
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
        let result =
            execute_in_pty("nonexistent_command_123", 1024, &pty_writer, &env, true)
                .expect("failed to execute");
        
        match result {
             ExecutionResult::Completed { exit_code, .. } => {
                 assert_ne!(exit_code, Some(0));
             }
             _ => panic!("Expected completion"),
        }
    }
}
