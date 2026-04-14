use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

/// A task definition from the tasks file.
#[derive(Debug, Deserialize, Clone)]
pub struct TaskDef {
    pub name: String,
    pub cwd: Option<String>,
    pub prompt: String,
    #[allow(dead_code)]
    pub budget: Option<f64>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub resume: Option<String>,
}

/// Task file containing a list of tasks.
#[derive(Debug, Deserialize)]
pub struct TaskFile {
    pub tasks: Vec<TaskDef>,
    #[allow(dead_code)]
    pub budget: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
enum TaskState {
    Pending,
    Running,
    Completed,
    Failed(String),
}

type SharedTail = Arc<Mutex<VecDeque<String>>>;

struct TaskRun {
    def: TaskDef,
    state: TaskState,
    pid: Option<u32>,
    start_time: Option<Instant>,
    child: Option<Child>,
    stdout_log: Option<PathBuf>,
    stderr_log: Option<PathBuf>,
    log_tail: SharedTail,
}

struct LaunchedTask {
    child: Child,
    stdout_log: PathBuf,
    stderr_log: PathBuf,
    log_tail: SharedTail,
}

/// Load tasks from a JSON file.
pub fn load_tasks(path: &str) -> io::Result<TaskFile> {
    let content = std::fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Run tasks with dependency resolution and parallel execution.
pub fn run_tasks(task_file: TaskFile, parallel: bool) -> io::Result<()> {
    let mut tasks: Vec<TaskRun> = task_file
        .tasks
        .into_iter()
        .map(|def| TaskRun {
            def,
            state: TaskState::Pending,
            pid: None,
            start_time: None,
            child: None,
            stdout_log: None,
            stderr_log: None,
            log_tail: Arc::new(Mutex::new(VecDeque::new())),
        })
        .collect();

    // Validate dependencies exist
    let names: Vec<String> = tasks.iter().map(|t| t.def.name.clone()).collect();
    for task in &tasks {
        for dep in &task.def.depends_on {
            if !names.contains(dep) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Task '{}' depends on '{}' which doesn't exist",
                        task.def.name, dep
                    ),
                ));
            }
        }
    }

    let total = tasks.len();
    println!("Running {total} tasks...");
    println!();

    let poll_interval = Duration::from_secs(2);
    let run_dir = create_run_dir()?;
    let print_lock = Arc::new(Mutex::new(()));
    println!("Logs: {}", run_dir.display());
    println!();

    loop {
        let completed: Vec<String> = tasks
            .iter()
            .filter(|t| t.state == TaskState::Completed)
            .map(|t| t.def.name.clone())
            .collect();

        let failed: Vec<String> = tasks
            .iter()
            .filter(|t| matches!(t.state, TaskState::Failed(_)))
            .map(|t| t.def.name.clone())
            .collect();

        let running_count = tasks
            .iter()
            .filter(|t| t.state == TaskState::Running)
            .count();
        let pending_count = tasks
            .iter()
            .filter(|t| t.state == TaskState::Pending)
            .count();

        print_status(&tasks);

        if completed.len() + failed.len() == total {
            println!();
            if failed.is_empty() {
                println!("All {total} tasks completed successfully.");
            } else {
                println!("{} completed, {} failed.", completed.len(), failed.len());
                for task in &tasks {
                    if let TaskState::Failed(ref msg) = task.state {
                        println!("  FAILED: {} — {}", task.def.name, msg);
                        print_task_logs(task);
                    }
                }
            }

            #[cfg(target_os = "macos")]
            {
                let msg = if failed.is_empty() {
                    format!("All {total} tasks completed")
                } else {
                    format!("{} completed, {} failed", completed.len(), failed.len())
                };
                let _ = Command::new("osascript")
                    .args([
                        "-e",
                        &format!("display notification \"{msg}\" with title \"claudectl run\""),
                    ])
                    .spawn();
            }

            return if failed.is_empty() {
                Ok(())
            } else {
                Err(io::Error::other(format!("{} tasks failed", failed.len())))
            };
        }

        for task in &mut tasks {
            if task.state != TaskState::Pending {
                continue;
            }

            let deps_met = task
                .def
                .depends_on
                .iter()
                .all(|dep| completed.contains(dep));
            let deps_failed = task.def.depends_on.iter().any(|dep| failed.contains(dep));

            if deps_failed {
                task.state = TaskState::Failed("dependency failed".into());
                continue;
            }

            if !deps_met {
                continue;
            }

            if !parallel && running_count > 0 {
                break;
            }

            match launch_claude_session(&task.def, &run_dir, Arc::clone(&print_lock)) {
                Ok(launched) => {
                    let pid = launched.child.id();
                    println!("  Started: {} (PID {})", task.def.name, pid);
                    println!(
                        "    logs: {}, {}",
                        launched.stdout_log.display(),
                        launched.stderr_log.display()
                    );
                    task.pid = Some(pid);
                    task.start_time = Some(Instant::now());
                    task.stdout_log = Some(launched.stdout_log);
                    task.stderr_log = Some(launched.stderr_log);
                    task.log_tail = launched.log_tail;
                    task.child = Some(launched.child);
                    task.state = TaskState::Running;
                }
                Err(e) => {
                    task.state = TaskState::Failed(format!("launch error: {e}"));
                }
            }
        }

        for task in &mut tasks {
            if task.state != TaskState::Running {
                continue;
            }

            let wait_result = if let Some(child) = task.child.as_mut() {
                child.try_wait()
            } else {
                Ok(None)
            };

            match wait_result {
                Ok(Some(status)) => {
                    let elapsed = task.start_time.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                    task.child = None;
                    if status.success() {
                        println!("  Finished: {} ({}s)", task.def.name, elapsed);
                        task.state = TaskState::Completed;
                    } else {
                        let reason = format!("exit {}", format_exit_status(status));
                        println!("  Failed: {} ({reason})", task.def.name);
                        print_task_tail(task);
                        task.state = TaskState::Failed(reason);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    let reason = format!("wait error: {e}");
                    println!("  Failed: {} ({reason})", task.def.name);
                    print_task_tail(task);
                    task.state = TaskState::Failed(reason);
                    task.child = None;
                }
            }
        }

        if pending_count > 0 && running_count == 0 {
            let launchable = tasks.iter().any(|t| {
                t.state == TaskState::Pending
                    && t.def.depends_on.iter().all(|dep| completed.contains(dep))
                    && !t.def.depends_on.iter().any(|dep| failed.contains(dep))
            });
            if !launchable {
                for task in &mut tasks {
                    if task.state == TaskState::Pending {
                        task.state = TaskState::Failed("unresolvable dependency".into());
                    }
                }
                continue;
            }
        }

        std::thread::sleep(poll_interval);
    }
}

fn launch_claude_session(
    task: &TaskDef,
    run_dir: &Path,
    print_lock: Arc<Mutex<()>>,
) -> io::Result<LaunchedTask> {
    let cwd = task.cwd.as_deref().unwrap_or(".");

    let mut args = vec!["--print".to_string()];
    if let Some(ref resume) = task.resume {
        args.push("--resume".into());
        args.push(resume.clone());
    }
    args.push(task.prompt.clone());

    let mut child = Command::new("claude")
        .args(&args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let slug = sanitize_task_name(&task.name);
    let stdout_log = run_dir.join(format!("{slug}.stdout.log"));
    let stderr_log = run_dir.join(format!("{slug}.stderr.log"));
    let log_tail = Arc::new(Mutex::new(VecDeque::new()));

    if let Some(stdout) = child.stdout.take() {
        spawn_log_pump(
            stdout,
            task.name.clone(),
            "stdout",
            stdout_log.clone(),
            Arc::clone(&log_tail),
            Arc::clone(&print_lock),
            false,
        );
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_log_pump(
            stderr,
            task.name.clone(),
            "stderr",
            stderr_log.clone(),
            Arc::clone(&log_tail),
            print_lock,
            true,
        );
    }

    Ok(LaunchedTask {
        child,
        stdout_log,
        stderr_log,
        log_tail,
    })
}

fn print_status(tasks: &[TaskRun]) {
    let total = tasks.len();
    let completed = tasks
        .iter()
        .filter(|t| t.state == TaskState::Completed)
        .count();
    let running = tasks
        .iter()
        .filter(|t| t.state == TaskState::Running)
        .count();
    let failed = tasks
        .iter()
        .filter(|t| matches!(t.state, TaskState::Failed(_)))
        .count();
    let pending = tasks
        .iter()
        .filter(|t| t.state == TaskState::Pending)
        .count();

    eprint!("\r  [{completed}/{total}] {running} running, {pending} pending, {failed} failed    ");
}

fn create_run_dir() -> io::Result<PathBuf> {
    let base = std::env::current_dir()?.join(".claudectl-runs");
    fs::create_dir_all(&base)?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let run_dir = base.join(format!("run-{now_ms}-{}", std::process::id()));
    fs::create_dir_all(&run_dir)?;
    Ok(run_dir)
}

fn sanitize_task_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('-');
        }
    }

    if out.is_empty() {
        "task".to_string()
    } else {
        out
    }
}

fn spawn_log_pump<R: std::io::Read + Send + 'static>(
    reader: R,
    task_name: String,
    stream_name: &'static str,
    log_path: PathBuf,
    log_tail: SharedTail,
    print_lock: Arc<Mutex<()>>,
    is_stderr: bool,
) {
    std::thread::spawn(move || {
        let mut log_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path)
            .ok();

        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            if let Some(file) = log_file.as_mut() {
                let _ = writeln!(file, "{line}");
            }
            push_tail(&log_tail, format!("[{stream_name}] {line}"));

            let _guard = print_lock.lock().ok();
            if is_stderr {
                eprintln!("\n[{}:{}] {}", task_name, stream_name, line);
            } else {
                println!("\n[{}] {}", task_name, line);
            }
        }
    });
}

fn push_tail(log_tail: &SharedTail, line: String) {
    let Ok(mut tail) = log_tail.lock() else {
        return;
    };
    tail.push_back(line);
    while tail.len() > 12 {
        tail.pop_front();
    }
}

fn format_exit_status(status: ExitStatus) -> String {
    status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_string())
}

fn print_task_tail(task: &TaskRun) {
    let Ok(tail) = task.log_tail.lock() else {
        return;
    };
    if tail.is_empty() {
        return;
    }

    println!("    recent output:");
    for line in tail.iter() {
        println!("      {line}");
    }
}

fn print_task_logs(task: &TaskRun) {
    if let Some(path) = &task.stdout_log {
        println!("    stdout: {}", path.display());
    }
    if let Some(path) = &task.stderr_log {
        println!("    stderr: {}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_tasks_json() {
        let json = r#"{
            "tasks": [
                {
                    "name": "task1",
                    "prompt": "Do something",
                    "cwd": "./src"
                },
                {
                    "name": "task2",
                    "prompt": "Do something else",
                    "depends_on": ["task1"],
                    "budget": 2.0
                }
            ],
            "budget": 10.0
        }"#;

        let task_file: TaskFile = serde_json::from_str(json).unwrap();
        assert_eq!(task_file.tasks.len(), 2);
        assert_eq!(task_file.tasks[0].name, "task1");
        assert_eq!(task_file.tasks[0].cwd, Some("./src".into()));
        assert_eq!(task_file.tasks[1].depends_on, vec!["task1"]);
        assert_eq!(task_file.tasks[1].budget, Some(2.0));
        assert_eq!(task_file.budget, Some(10.0));
    }

    #[test]
    fn test_dependency_validation() {
        let task_file = TaskFile {
            tasks: vec![TaskDef {
                name: "task1".into(),
                prompt: "test".into(),
                cwd: None,
                budget: None,
                depends_on: vec!["nonexistent".into()],
                resume: None,
            }],
            budget: None,
        };

        let result = run_tasks(task_file, false);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }

    #[test]
    fn test_sanitize_task_name() {
        assert_eq!(sanitize_task_name("Update docs"), "update-docs");
        assert_eq!(sanitize_task_name("API/Test #1"), "apitest-1");
    }
}
