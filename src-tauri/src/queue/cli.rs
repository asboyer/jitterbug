use super::*;

pub const HELP: &str = "Shared validation queue (keep the Jitterbug desktop open):
  jitterbug-run queue add NAME --worktree PATH [--memory-mib 2048] -- EXECUTABLE [ARG ...]
  jitterbug-run queue remove NAME
  jitterbug-run queue submit NAME [--task TITLE] [--harness NAME] [--session ID] [--request-id ID]
  jitterbug-run queue run NAME [same options]  # wait, print output, preserve exit status
  jitterbug-run queue status
  jitterbug-run queue result JOB_ID
  jitterbug-run queue cancel JOB_ID           # queued jobs only
  jitterbug-run queue pause|resume
  jitterbug-run queue release JOB_ID --reviewed-descendants
  jitterbug-run mcp --profiles NAME,NAME [--harness claude|codex|meta]

Profiles are explicit permission to run a fixed command in a fixed worktree. Inspect the
command before registering it. MCP cannot register profiles or supply shell commands.
The single slot is shared across all clients of this OS user. Only submitted jobs are
governed. Running jobs are never killed automatically. After a worker crash, inspect
its descendants before releasing its slot. Memory estimates are admission hints, not
hard limits. Git fingerprints exclude ignored files and external inputs. Outputs are
bounded and private on disk; requesting output through MCP shares it with that client.
JITTERBUG_QUEUE_DIR overrides the local registry for isolated development and tests.";

fn executable(name: &str) -> Result<PathBuf, String> {
    let direct = PathBuf::from(name);
    if direct.components().count() > 1 || direct.is_absolute() {
        let path = if direct.is_absolute() {
            direct
        } else {
            std::env::current_dir()
                .map_err(|_| "Working directory unavailable.")?
                .join(direct)
        };
        return if path.is_file() {
            Ok(path)
        } else {
            Err("Executable not found.".into())
        };
    }
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        #[cfg(windows)]
        let suffixes = ["", ".exe", ".cmd", ".bat"];
        #[cfg(not(windows))]
        let suffixes = [""];
        for suffix in suffixes {
            let path = directory.join(format!("{name}{suffix}"));
            if path.is_file() {
                // Keep symlink entrypoints (notably Python virtual environments).
                return if path.is_absolute() {
                    Ok(path)
                } else {
                    Ok(std::env::current_dir()
                        .map_err(|_| "Working directory unavailable.")?
                        .join(path))
                };
            }
        }
    }
    Err("Executable not found on PATH.".into())
}

pub fn run(args: &[String]) -> Result<i32, String> {
    let directory = directory()?;
    let action = args.first().map(String::as_str).unwrap_or("help");
    let at = |i| {
        args.get(i)
            .cloned()
            .ok_or_else(|| "Missing argument; use queue --help.".to_string())
    };
    match action {
        "help" | "--help" | "-h" => println!("{HELP}"),
        "add" => {
            let name = at(1)?;
            let mut cwd = None;
            let mut memory = 2048u64;
            let mut i = 2;
            while i < args.len() && args[i] != "--" {
                match args[i].as_str() {
                    "--worktree" => {
                        cwd = Some(
                            PathBuf::from(at(i + 1)?)
                                .canonicalize()
                                .map_err(|_| "Worktree unavailable.")?,
                        )
                    }
                    "--memory-mib" => {
                        memory = at(i + 1)?.parse().map_err(|_| "Invalid memory estimate.")?
                    }
                    _ => return Err("Unknown profile option.".into()),
                }
                i += 2;
            }
            if args.get(i).is_none_or(|a| a != "--") {
                return Err("Supply the approved command after --.".into());
            }
            let mut command = args.get(i + 1..).unwrap_or_default().to_vec();
            let first = command.first_mut().ok_or("Missing executable.")?;
            *first = executable(first)?
                .to_str()
                .ok_or("Executable must be UTF-8.")?
                .into();
            add_profile(
                &directory,
                Profile {
                    name,
                    worktree: cwd.ok_or("--worktree is required.")?,
                    command,
                    memory_bytes: memory
                        .checked_mul(1024 * 1024)
                        .ok_or("Memory estimate too large.")?,
                    search_path: std::env::var("PATH").ok(),
                },
            )?;
            println!("Approved validation profile saved locally.");
        }
        "remove" => {
            let name = at(1)?;
            transaction(&directory, |s| {
                s.profiles.retain(|p| p.name != name);
                Ok(())
            })?;
            println!("Profile removed. Waiting jobs will not run under a removed profile.");
        }
        "submit" | "run" => {
            let profile = at(1)?;
            let mut task = profile.clone();
            let mut harness = "manual".to_string();
            let mut session = "terminal".to_string();
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let mut request = format!("cli-{}-{unique:x}", std::process::id());
            let mut i = 2;
            while i < args.len() {
                let value = at(i + 1)?;
                match args[i].as_str() {
                    "--task" => task = value,
                    "--harness" => harness = value,
                    "--session" => session = value,
                    "--request-id" => request = value,
                    _ => return Err("Unknown submission option.".into()),
                }
                i += 2;
            }
            let submitted = submit(&directory, &profile, &request, &task, &harness, &session)?;
            if action == "submit" {
                println!(
                    "{}",
                    serde_json::to_string(&submitted).map_err(|_| "Cannot encode job.")?
                );
            } else {
                eprintln!("Queued {}. Keep Jitterbug open; Ctrl-C stops waiting, not the job. Cancel queued work explicitly.", submitted.id);
                loop {
                    let state = inspect(&directory)?;
                    let job = state
                        .jobs
                        .iter()
                        .find(|j| j.id == submitted.id)
                        .ok_or("Job no longer retained.")?;
                    if !job.stage.active() {
                        let output = worker::output(&directory, &job.id)?;
                        print!("{}", output.stdout);
                        eprint!("{}", output.stderr);
                        eprintln!(
                            "\n{}{}",
                            job.message,
                            if output.truncated {
                                " Output truncated to 32 KiB per stream."
                            } else {
                                ""
                            }
                        );
                        return Ok(
                            if job.stage == Stage::Passed || job.stage == Stage::Failed {
                                job.exit_code.unwrap_or(2)
                            } else {
                                3
                            },
                        );
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
        "status" => println!(
            "{}",
            serde_json::to_string_pretty(&inspect(&directory)?.view())
                .map_err(|_| "Cannot encode queue.")?
        ),
        "result" => {
            let id = at(1)?;
            let state = inspect(&directory)?;
            let job = state
                .jobs
                .iter()
                .find(|j| j.id == id)
                .ok_or("Job not found.")?;
            println!(
                "{}",
                serde_json::json!({"job": JobView::from(job), "output": worker::output(&directory, &id)?})
            );
        }
        "cancel" => cancel(&directory, &at(1)?)?,
        "pause" | "resume" => transaction(&directory, |s| {
            s.paused = action == "pause";
            Ok(())
        })?,
        "release" => {
            if args.get(2).is_none_or(|a| a != "--reviewed-descendants") {
                return Err("Review surviving descendant processes first, then supply --reviewed-descendants. No processes will be stopped.".into());
            }
            acknowledge(&directory, &at(1)?)?;
        }
        _ => return Err("Unknown queue command; use queue --help.".into()),
    }
    Ok(0)
}
