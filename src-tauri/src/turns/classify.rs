//! Which shell commands are heavy: test suites, builds, and type checks that saturate cores or
//! hold gigabytes for minutes. Everything else (git, ls, grep, edits, installs) passes at once.
//! Deterministic on purpose: the gate runs before every command and must answer in milliseconds.

/// Program → subcommands that make it heavy. An empty list means the program is always heavy.
const HEAVY: &[(&str, &[&str])] = &[
    (
        "cargo",
        &[
            "build", "test", "check", "clippy", "bench", "nextest", "run", "doc",
        ],
    ),
    ("npm", &["test", "t", "run", "build", "exec"]),
    ("pnpm", &["test", "t", "run", "build", "exec"]),
    ("yarn", &["test", "run", "build"]),
    ("bun", &["test", "run", "build"]),
    (
        "npx",
        &[
            "vitest",
            "jest",
            "playwright",
            "tsc",
            "next",
            "vite",
            "webpack",
            "svelte-check",
        ],
    ),
    ("pytest", &[]),
    ("py.test", &[]),
    ("tox", &[]),
    ("nox", &[]),
    ("go", &["test", "build", "vet"]),
    ("swift", &["build", "test"]),
    ("xcodebuild", &[]),
    ("gradle", &[]),
    ("gradlew", &[]),
    ("mvn", &[]),
    ("make", &[]),
    ("ninja", &[]),
    ("bazel", &["build", "test", "run"]),
    ("cmake", &["--build"]),
    ("docker", &["build", "compose"]),
    ("tsc", &[]),
    ("vitest", &[]),
    ("jest", &[]),
    ("playwright", &["test"]),
    ("svelte-check", &[]),
    ("webpack", &[]),
    ("dotnet", &["build", "test"]),
    ("mix", &["test", "compile"]),
    ("rspec", &[]),
    ("phpunit", &[]),
];

/// Scripts named in `npm run X` that are long-running servers, not finite work.
const SERVERS: &[&str] = &["dev", "start", "serve", "watch", "preview", "storybook"];

/// Wrappers that run the next word as the real command.
const WRAPPERS: &[&str] = &[
    "sudo", "env", "time", "nice", "nohup", "uv", "poetry", "pipenv",
];

/// The display name of a heavy command (for example `cargo test` or `npm run build`), or `None`
/// when the command is light.
pub fn heavy(command: &str) -> Option<String> {
    command
        .split(['\n', ';', '|', '&'])
        .find_map(|segment| heavy_segment(segment.trim()))
}

fn heavy_segment(segment: &str) -> Option<String> {
    let words: Vec<&str> = segment
        .split_whitespace()
        .map(|w| w.trim_matches(|c| matches!(c, '"' | '\'' | '(' | ')')))
        .filter(|w| !w.is_empty())
        .collect();
    let mut index = 0;
    // Skip variable assignments, wrappers, and their flags (`env FOO=1 nice -n 5 cargo test`).
    while index < words.len() {
        let word = words[index];
        let name = program(word);
        if word.contains('=') && !word.starts_with('-') {
            index += 1;
        } else if WRAPPERS.contains(&name.as_str()) || (name == "run" && index > 0) {
            index += 1;
            while index < words.len() && words[index].starts_with('-') {
                index += 1;
            }
        } else {
            break;
        }
    }
    let rest = &words[index.min(words.len())..];
    let first = program(rest.first()?);
    // `bash -lc "npm test"` and similar: classify the quoted command.
    if matches!(first.as_str(), "bash" | "sh" | "zsh") {
        let inner = rest.iter().skip(1).skip_while(|w| w.starts_with('-'));
        let inner: Vec<&str> = inner.copied().collect();
        return (!inner.is_empty())
            .then(|| heavy(&inner.join(" ")))
            .flatten();
    }
    // `python -m pytest`, `python3 -m unittest`.
    if first.starts_with("python") {
        let module = rest.windows(2).find(|w| w[0] == "-m").map(|w| w[1])?;
        return matches!(module, "pytest" | "unittest" | "tox" | "nox")
            .then(|| format!("{first} -m {module}"));
    }
    let (_, subcommands) = HEAVY.iter().find(|(name, _)| *name == first)?;
    if subcommands.is_empty() {
        return Some(first);
    }
    let sub = rest
        .iter()
        .skip(1)
        .find(|w| !w.starts_with('-') || *w == &"--build")?;
    if !subcommands.contains(sub) {
        return None;
    }
    if matches!(first.as_str(), "npm" | "pnpm" | "yarn" | "bun") && *sub == "run" {
        let script = rest.iter().skip(2).find(|w| !w.starts_with('-'))?;
        if SERVERS.iter().any(|s| script.starts_with(s)) {
            return None;
        }
        return Some(format!("{first} run {}", short(script)));
    }
    if matches!(first.as_str(), "npm" | "pnpm") && *sub == "exec" {
        let tool = rest.iter().skip(2).find(|w| !w.starts_with('-'))?;
        return heavy(tool).map(|_| format!("{first} exec {}", short(tool)));
    }
    if first == "yarn" && !matches!(*sub, "test" | "build" | "run") {
        return None;
    }
    if first == "docker" && *sub == "compose" {
        return rest
            .iter()
            .any(|w| *w == "build" || *w == "--build")
            .then(|| "docker compose build".into());
    }
    Some(format!("{first} {}", short(sub)))
}

/// The bare program name, without directories or a Windows extension.
fn program(word: &str) -> String {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let name = name.strip_suffix(".exe").unwrap_or(name);
    name.strip_prefix("./").unwrap_or(name).to_ascii_lowercase()
}

fn short(word: &str) -> String {
    word.chars().filter(|c| !c.is_control()).take(40).collect()
}

#[cfg(test)]
mod tests {
    use super::heavy;

    #[test]
    fn test_suites_and_builds_are_heavy() {
        for (command, name) in [
            ("cargo test --lib", "cargo test"),
            ("cd app && npm test", "npm test"),
            ("npm run build", "npm run build"),
            ("pnpm run test:unit -- --watch=false", "pnpm run test:unit"),
            ("pytest -x tests/", "pytest"),
            ("python3 -m pytest -q", "python3 -m pytest"),
            ("RUST_LOG=1 cargo build --release", "cargo build"),
            ("bash -lc 'go test ./...'", "go test"),
            ("/usr/bin/make -j8", "make"),
            ("npx vitest run", "npx vitest"),
            ("uv run pytest", "pytest"),
            ("docker compose up --build", "docker compose build"),
        ] {
            assert_eq!(heavy(command).as_deref(), Some(name), "{command}");
        }
    }

    #[test]
    fn everyday_commands_and_servers_pass() {
        for command in [
            "git status",
            "ls -la",
            "rg TODO src",
            "npm install",
            "npm run dev",
            "cargo fmt",
            "python3 script.py",
            "go version",
            "docker ps",
            "cat Cargo.toml | head",
        ] {
            assert_eq!(heavy(command), None, "{command}");
        }
    }
}
