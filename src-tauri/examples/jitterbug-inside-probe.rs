//! Opt-in check of the browser inside view against an isolated, headless Chrome profile.
//! Opens one busy tab and two idle tabs, expects the busy tab to be matched to its own renderer
//! and ranked first, then closes it through the tab action. Prints only probe-tab measurements.
//! `--sessions` lists commands inside running terminal/editor apps by command name only.
use std::{process::Command, time::Duration};

const CHROME: &str = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const BUSY: &str = "data:text/html,<title>jitterbug-busy</title><script>const keep=[];for(let i=0;i<2e6;i++)keep.push({i,s:'x'+i});setInterval(()=>{const t=performance.now();while(performance.now()-t<60);},100)</script>";
const IDLE: &str = "data:text/html,<title>jitterbug-idle</title>";

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if std::env::args().any(|a| a == "--sessions") {
        return sessions();
    }
    let profile = std::env::temp_dir().join(format!("jitterbug-inside-{}", std::process::id()));
    let mut chrome = Command::new(CHROME)
        .args([
            "--headless=new",
            "--remote-debugging-port=0",
            "--no-first-run",
            "--no-default-browser-check",
            "about:blank",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("Chrome is required for this probe");
    let result = run(&profile, chrome.id()).await;
    let _ = chrome.kill();
    let _ = chrome.wait();
    let _ = std::fs::remove_dir_all(&profile);
    if let Err(e) = result {
        eprintln!("Inside probe failed: {e}");
        std::process::exit(1);
    }
}

async fn run(profile: &std::path::Path, browser: u32) -> Result<(), String> {
    let port_file = profile.join("DevToolsActivePort");
    let mut port = None;
    for _ in 0..50 {
        if let Some(p) = std::fs::read_to_string(&port_file)
            .ok()
            .and_then(|t| t.lines().next().and_then(|l| l.trim().parse::<u16>().ok()))
        {
            port = Some(p);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let port = port.ok_or("Chrome did not open a debugging port")?;
    std::env::set_var("JITTERBUG_BROWSER_PROFILES", profile);
    let http = reqwest::Client::new();
    for url in [BUSY, IDLE, IDLE] {
        http.put(format!(
            "http://127.0.0.1:{port}/json/new?{}",
            url.replace('%', "%25").replace('#', "%23")
        ))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    }
    let mut collector = jitterbug_lib::collector::Collector::default();
    tokio::time::sleep(Duration::from_secs(3)).await;
    collector.sample();
    tokio::time::sleep(Duration::from_secs(5)).await;
    let snapshot = collector.sample();
    let workload = snapshot
        .processes
        .iter()
        .find(|p| p.pid == browser)
        .map(|p| p.workload_id.clone())
        .ok_or("Probe browser missing from the inventory")?;
    let inside = jitterbug_lib::inside::inspect(&snapshot, &workload)?;
    if inside.status != "ready" {
        return Err(format!("{}: {}", inside.status, inside.message));
    }
    let probe: Vec<_> = inside
        .units
        .iter()
        .filter(|u| u.title.starts_with("jitterbug-"))
        .collect();
    for u in &probe {
        println!(
            "{:<15} cpu {:>5.2}% of machine  memory {:>7}  shared_with {}  {}",
            u.title,
            u.cpu_percent,
            jitterbug_lib::inside::size(u.memory_bytes),
            u.shared_with,
            u.confidence
        );
    }
    let busy = probe
        .iter()
        .find(|u| u.title == "jitterbug-busy")
        .ok_or("Busy tab not listed")?;
    let idle_max = probe
        .iter()
        .filter(|u| u.title == "jitterbug-idle")
        .map(|u| u.cpu_percent)
        .fold(0.0, f32::max);
    if busy.pid.is_none() || busy.confidence != "Matched by CPU time" {
        return Err(format!(
            "Busy tab not matched to a renderer: {}",
            busy.confidence
        ));
    }
    if busy.cpu_percent <= idle_max * 3.0 || busy.memory_bytes < 100 << 20 {
        return Err("Busy tab was not clearly attributed more CPU and memory".into());
    }
    if probe.first().map(|u| u.title.as_str()) != Some("jitterbug-busy") {
        return Err("Busy tab is not ranked first among probe tabs".into());
    }
    println!(
        "Remainder: {}",
        inside
            .remainder
            .iter()
            .map(|r| format!("{} ×{}", r.label, r.process_count))
            .collect::<Vec<_>>()
            .join("; ")
    );
    // The incident path: a memory alert on the browser offers closing the heavy tab, not
    // quitting the browser. The connection opened above is reused, as after "Show tabs".
    let option = jitterbug_lib::inside::options::option(&snapshot, &workload, true)
        .ok_or("No tab was offered for a memory incident")?;
    let unit = option.unit.as_ref().ok_or("The offer has no tab")?;
    println!(
        "Incident alternative: {} → “{}” ({})",
        option.action,
        unit.title,
        jitterbug_lib::inside::size(unit.memory_bytes)
    );
    if option.action != "close_tab" || unit.id != busy.id {
        return Err("The memory incident did not offer closing the heavy tab".into());
    }
    let message = jitterbug_lib::inside::chrome::act(&unit.id, &unit.detail, "close_tab")?;
    println!("{message}");
    if jitterbug_lib::inside::chrome::act(&busy.id, &busy.detail, "focus_tab").is_ok() {
        return Err("Closed tab is still open".into());
    }
    println!(
        "Inside probe passed: busy tab matched, ranked first, offered for closing, and closed."
    );
    Ok(())
}

fn sessions() {
    let mut collector = jitterbug_lib::collector::Collector::default();
    collector.sample();
    std::thread::sleep(Duration::from_secs(5));
    let snapshot = collector.sample();
    for workload in &snapshot.workloads {
        let Ok(inside) = jitterbug_lib::inside::inspect(&snapshot, &workload.id) else {
            continue;
        };
        if inside.source != "Process tree" {
            continue;
        }
        println!("{}: {}", workload.label, inside.message);
        for u in &inside.units {
            println!(
                "  {:<12} cpu {:>5.1}%  memory {:>8}  {}  actions {:?}",
                u.host.as_deref().unwrap_or("?"),
                u.cpu_percent,
                jitterbug_lib::inside::size(u.memory_bytes),
                u.evidence[0],
                u.actions
            );
        }
        for r in &inside.remainder {
            println!("  ({}: {} process(es))", r.label, r.process_count);
        }
    }
}
