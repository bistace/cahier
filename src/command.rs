use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::io::Write;
use std::time::Instant;

use crate::db;
use crate::executor::{self, Job, ExecutionResult};
use crate::prompt::CahierPrompt;

pub struct CommandContext<'a> {
    pub db: &'a db::Database,
    pub current_env: &'a Arc<Mutex<HashMap<String, String>>>,
    pub jobs: &'a mut Vec<Job>,
    pub pty_writer: &'a Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    pub max_output_size: usize,
    pub prompt: &'a mut CahierPrompt,
    pub aliases: &'a Arc<Mutex<HashMap<String, String>>>,
    pub should_log: bool,
    pub db_path: &'a str,
    pub next_command: &'a mut Option<String>,
}

pub enum CommandResult {
    Continue,
    Exit,
}

pub trait Command: Send + Sync {
    fn name(&self) -> &str;
    fn execute(&self, args: &[&str], context: &mut CommandContext) -> Result<CommandResult>;
}

pub struct Registry {
    commands: HashMap<String, Box<dyn Command>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
        }
    }
    
    pub fn register(&mut self, cmd: Box<dyn Command>) {
        self.commands.insert(cmd.name().to_string(), cmd);
    }
    
    pub fn get(&self, name: &str) -> Option<&dyn Command> {
        self.commands.get(name).map(|b| b.as_ref())
    }

    pub fn command_names(&self) -> Vec<String> {
        self.commands.keys().cloned().collect()
    }
}

// Built-in commands

pub struct CdCommand;
impl Command for CdCommand {
    fn name(&self) -> &str { "cd" }
    fn execute(&self, args: &[&str], context: &mut CommandContext) -> Result<CommandResult> {
        let current_pwd = std::env::current_dir().ok();

        let path = if args.is_empty() {
            dirs::home_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string())
        } else if args[0] == "-" {
            let env = context.current_env.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(old) = env.get("OLDPWD") {
                println!("{}", old);
                old.clone()
            } else {
                eprintln!("cd: OLDPWD not set");
                context.prompt.set_last_success(false);
                context.prompt.set_last_duration(Some(std::time::Duration::from_millis(0)));
                return Ok(CommandResult::Continue);
            }
        } else {
            args[0].to_string()
        };

        let start = Instant::now();
        if let Err(e) = std::env::set_current_dir(&path) {
             eprintln!("Error changing directory: {}", e);
             context.prompt.set_last_success(false);
        } else {
            if let Ok(cwd) = std::env::current_dir() {
                if let Some(cwd_str) = cwd.to_str() {
                    // Use robust locking
                    match context.current_env.lock() {
                        Ok(mut env) => {
                             env.insert("PWD".to_string(), cwd_str.to_string());
                             if let Some(prev) = current_pwd {
                                 env.insert("OLDPWD".to_string(), prev.to_string_lossy().to_string());
                             }
                        },
                        Err(_) => eprintln!("Warning: Failed to lock env to update PWD"),
                    }
                }
            }
            
            let cmd_line = format!("cd {}", path);
            if context.should_log {
                if let Err(e) = context.db.log_entry(&cmd_line, "", Some(0), 0, None) {
                    eprintln!("Error logging cd command: {}", e);
                }
            }
            context.prompt.set_last_success(true);
        }
        
        context.prompt.set_last_duration(Some(start.elapsed()));
        
        Ok(CommandResult::Continue)
    }
}

pub struct JobsCommand;
impl Command for JobsCommand {
    fn name(&self) -> &str { "jobs" }
    fn execute(&self, _args: &[&str], context: &mut CommandContext) -> Result<CommandResult> {
        let start = Instant::now();
        for (i, job) in context.jobs.iter().enumerate() {
            println!("[{}] {} {}", i + 1, job.command, if i == context.jobs.len() - 1 { "+" } else { "" });
        }
        context.prompt.set_last_success(true);
        context.prompt.set_last_duration(Some(start.elapsed()));
        Ok(CommandResult::Continue)
    }
}

pub struct ExitCommand;
impl Command for ExitCommand {
    fn name(&self) -> &str { "exit" }
    fn execute(&self, _args: &[&str], _context: &mut CommandContext) -> Result<CommandResult> {
        Ok(CommandResult::Exit)
    }
}

pub struct FgCommand;
impl Command for FgCommand {
    fn name(&self) -> &str { "fg" }
    fn execute(&self, args: &[&str], context: &mut CommandContext) -> Result<CommandResult> {
        let start_total = Instant::now();
        
        if context.jobs.is_empty() {
            eprintln!("fg: current: no such job");
            context.prompt.set_last_success(false);
            context.prompt.set_last_duration(Some(start_total.elapsed()));
            return Ok(CommandResult::Continue);
        }

        let job_index = if args.is_empty() {
            context.jobs.len() - 1
        } else {
            let arg = args[0];
            if let Ok(n) = arg.parse::<usize>() {
                 if n > 0 && n <= context.jobs.len() {
                     n - 1
                 } else {
                     eprintln!("fg: {}: no such job", arg);
                     context.prompt.set_last_success(false);
                     context.prompt.set_last_duration(Some(start_total.elapsed()));
                     return Ok(CommandResult::Continue);
                 }
            } else {
                eprintln!("fg: invalid job specifier");
                context.prompt.set_last_success(false);
                context.prompt.set_last_duration(Some(start_total.elapsed()));
                return Ok(CommandResult::Continue);
            }
        };

        let job = context.jobs.remove(job_index);
        let job_command = job.command.clone();
        println!("{}", job.command);

        let start = Instant::now();
        match executor::resume_job(job, context.max_output_size, context.pty_writer, context.current_env) {
            Ok(res) => {
                handle_execution_result(res, start, &job_command, context)?;
            }
            Err(e) => {
                eprintln!("Error resuming job: {}", e);
                context.prompt.set_last_success(false);
                context.prompt.set_last_duration(Some(start.elapsed()));
            }
        }
        
        Ok(CommandResult::Continue)
    }
}

pub struct AliasCommand;
impl Command for AliasCommand {
    fn name(&self) -> &str { "alias" }
    fn execute(&self, args: &[&str], context: &mut CommandContext) -> Result<CommandResult> {
        let start = Instant::now();
        
        let mut aliases_guard = context.aliases.lock()
            .map_err(|_| anyhow::anyhow!("Failed to lock aliases registry"))?;

        let mut all_success = true;
        if args.is_empty() {
            // List all aliases
            for (name, value) in aliases_guard.iter() {
                println!("alias {}='{}'", name, value);
            }
        } else {
            // Define new aliases
            for arg in args {
                if let Some((name, value)) = arg.split_once('=') {
                    let name = name.trim().to_string();
                    let value = value.trim().to_string(); // Simplification: value should be stripped of quotes if present, but shlex handles that
                    
                    if !name.is_empty() {
                        aliases_guard.insert(name, value);
                    }
                } else {
                     // If only name provided, show that alias
                     if let Some(value) = aliases_guard.get(*arg) {
                         println!("alias {}='{}'", arg, value);
                     } else {
                         eprintln!("alias: {}: not found", arg);
                         all_success = false;
                     }
                }
            }
        }
        
        context.prompt.set_last_success(all_success);
        context.prompt.set_last_duration(Some(start.elapsed()));
        Ok(CommandResult::Continue)
    }
}

pub struct UnaliasCommand;
impl Command for UnaliasCommand {
    fn name(&self) -> &str { "unalias" }
    fn execute(&self, args: &[&str], context: &mut CommandContext) -> Result<CommandResult> {
        let start = Instant::now();
        let mut aliases_guard = context.aliases.lock()
            .map_err(|_| anyhow::anyhow!("Failed to lock aliases registry"))?;

        let mut all_success = true;
        if args.is_empty() {
            eprintln!("unalias: usage: unalias name [name ...]");
            all_success = false;
        } else {
            for arg in args {
                if aliases_guard.remove(*arg).is_none() {
                    eprintln!("unalias: {}: not found", arg);
                    all_success = false;
                }
            }
        }
        
        context.prompt.set_last_success(all_success);
        context.prompt.set_last_duration(Some(start.elapsed()));
        Ok(CommandResult::Continue)
    }
}

pub struct EditCommand;
impl Command for EditCommand {
    fn name(&self) -> &str { "edit" }
    fn execute(&self, _args: &[&str], context: &mut CommandContext) -> Result<CommandResult> {
        let start = Instant::now();
        
        // Open a new connection for the TUI to avoid conflict or ownership issues,
        // and because TUI takes ownership of the DB instance in its current signature.
        let db = db::Database::init(context.db_path)?;
        
        if let Some(cmd) = crate::tui::run(db)? {
            *context.next_command = Some(cmd);
        }
        
        // Force refresh of current context db connection if needed? 
        // The current architecture passes &db reference to commands, so we can't easily replace it.
        // However, SQLite handles concurrent connections to the same file fine.
        // The TUI might have modified the DB, but since we query fresh on every command usually, it should be fine.
        
        context.prompt.set_last_success(true);
        context.prompt.set_last_duration(Some(start.elapsed()));
        Ok(CommandResult::Continue)
    }
}


pub fn handle_execution_result(
    res: ExecutionResult,
    start_time: Instant,
    input: &str,
    context: &mut CommandContext
) -> Result<()> {
    match res {
        ExecutionResult::Completed { output, exit_code, output_file } => {
            let duration = start_time.elapsed();
            if context.should_log {
                if let Err(e) = context.db.log_entry(
                    input,
                    &output,
                    exit_code,
                    duration.as_millis(),
                    output_file.as_deref(),
                ) {
                    eprintln!("Error logging command: {}", e);
                }
            }
            
            if let Some(code) = exit_code {
                context.prompt.set_last_success(code == 0);
            } else {
                context.prompt.set_last_success(false);
            }
            context.prompt.set_last_duration(Some(duration));
        }
        ExecutionResult::Suspended(mut job) => {
             let id = context.jobs.len() + 1;
             job.id = id;
             println!("\n[{}] Stopped  {}", id, job.command);
             context.jobs.push(job);
             context.prompt.set_last_success(true);
             context.prompt.set_last_duration(Some(start_time.elapsed()));
        }
    }
    Ok(())
}
