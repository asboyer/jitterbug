//! Read-only monitor inventory and batched, reusable executable descriptions.
//! No task management or scheduling lives here. Private command arguments stay local.
use crate::{details, profiles, supervisor::Runtime, types::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Mutex,
};

type HistoryEntry = ((String, u32, u64), Vec<ResourcePoint>);
static PROCESS_HISTORY: Mutex<VecDeque<HistoryEntry>> = Mutex::new(VecDeque::new());

pub fn observe(snapshot: &Snapshot, process: &ProcessRecord) -> Vec<ResourcePoint> {
    let Ok(mut entries) = PROCESS_HISTORY.lock() else {
        return vec![];
    };
    record(&mut entries, snapshot, process)
}

fn record(
    entries: &mut VecDeque<HistoryEntry>,
    snapshot: &Snapshot,
    process: &ProcessRecord,
) -> Vec<ResourcePoint> {
    entries.retain(|(_, points)| {
        points
            .last()
            .is_some_and(|p| p.time + 600 >= snapshot.captured_at)
    });
    let key = (process.workload_id.clone(), process.pid, process.start_time);
    let mut points = entries
        .iter()
        .position(|(id, _)| id == &key)
        .and_then(|i| entries.remove(i))
        .map(|(_, p)| p)
        .unwrap_or_default();
    if snapshot.sample_ready && points.last().is_none_or(|p| p.time < snapshot.captured_at) {
        points.push(ResourcePoint {
            time: snapshot.captured_at,
            cpu_percent: process.cpu_percent,
            memory_bytes: process.memory_bytes,
            unresponsive: process.unresponsive,
        });
        if points.len() > 24 {
            points.remove(0);
        }
    }
    entries.push_back((key, points.clone()));
    while entries.len() > 24 {
        entries.pop_front();
    }
    points
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Identity {
    pub workload_id: String,
    pub pid: u32,
    pub start_time: u64,
}

#[derive(Deserialize)]
pub struct Query {
    pub search: String,
    pub view: String,
    pub sort: String,
    pub filter: String,
    pub offset: usize,
}

#[derive(Clone, Serialize)]
pub struct Row {
    pub workload: Workload,
    pub process: Option<ProcessRecord>,
    pub identity: Option<Identity>,
}

#[derive(Serialize)]
pub struct Inventory {
    pub rows: Vec<Row>,
    pub total: usize,
    pub offset: usize,
    pub captured_at: u64,
    pub observed_processes: usize,
    pub system_processes: usize,
}

pub fn inventory(snapshot: &Snapshot, query: &Query) -> Result<Inventory, String> {
    if query.search.len() > 128
        || !matches!(query.view.as_str(), "apps" | "processes")
        || !matches!(query.sort.as_str(), "impact" | "cpu" | "memory" | "name")
        || !matches!(query.filter.as_str(), "all" | "apps" | "system")
    {
        return Err("Choose a supported monitor filter.".into());
    }
    let search = query.search.trim().to_lowercase();
    let matches_group = |w: &Workload| match query.filter.as_str() {
        "apps" => matches!(
            w.category.as_str(),
            "application" | "app" | "browser" | "editor"
        ),
        "system" => w.category == "system",
        _ => true,
    };
    let identity = |p: &ProcessRecord| Identity {
        workload_id: p.workload_id.clone(),
        pid: p.pid,
        start_time: p.start_time,
    };
    let mut rows = Vec::new();
    for workload in &snapshot.workloads {
        if !matches_group(workload) {
            continue;
        }
        let members: Vec<_> = snapshot
            .processes
            .iter()
            .filter(|p| p.workload_id == workload.id)
            .collect();
        let group_match = search.is_empty() || workload.label.to_lowercase().contains(&search);
        let process_match = |p: &&ProcessRecord| {
            p.name.to_lowercase().contains(&search) || p.pid.to_string() == search
        };
        if query.view == "apps" {
            if group_match || members.iter().any(process_match) {
                rows.push(Row {
                    workload: workload.clone(),
                    process: None,
                    identity: (!members.is_empty())
                        .then(|| identity(details::representative(&members))),
                });
            }
        } else {
            for process in members.iter().filter(|p| group_match || process_match(p)) {
                rows.push(Row {
                    workload: workload.clone(),
                    process: Some((*process).clone()),
                    identity: Some(identity(process)),
                });
            }
        }
    }
    let readings = |r: &Row| {
        r.process
            .as_ref()
            .map_or((r.workload.cpu_percent, r.workload.memory_bytes), |p| {
                (p.cpu_percent, p.memory_bytes)
            })
    };
    let name = |r: &Row| {
        r.process
            .as_ref()
            .map_or(r.workload.label.to_lowercase(), |p| p.name.to_lowercase())
    };
    rows.sort_by(|a, b| {
        let (ac, am) = readings(a);
        let (bc, bm) = readings(b);
        let order = match query.sort.as_str() {
            "cpu" => bc.total_cmp(&ac),
            "memory" => bm.cmp(&am),
            "name" => name(a).cmp(&name(b)),
            _ => (bc + bm as f32 / snapshot.memory_total_bytes.max(1) as f32 * 100.0)
                .total_cmp(&(ac + am as f32 / snapshot.memory_total_bytes.max(1) as f32 * 100.0)),
        };
        order.then_with(|| name(a).cmp(&name(b))).then_with(|| {
            a.identity
                .as_ref()
                .map(|p| p.pid)
                .cmp(&b.identity.as_ref().map(|p| p.pid))
        })
    });
    let total = rows.len();
    let offset = query.offset.min(total.saturating_sub(1) / 40 * 40);
    Ok(Inventory {
        rows: rows.into_iter().skip(offset).take(40).collect(),
        total,
        offset,
        captured_at: snapshot.captured_at,
        observed_processes: snapshot.processes.len(),
        system_processes: snapshot.process_count,
    })
}

#[derive(Serialize)]
pub struct Explanation {
    pub identity: Identity,
    pub profile: profiles::Profile,
}

impl Runtime {
    pub async fn monitor_profiles(
        &self,
        identities: Vec<Identity>,
        generate: bool,
    ) -> Result<Vec<Explanation>, String> {
        if identities.len() > 6 {
            return Err("Explain at most six visible processes at a time.".into());
        }
        // Cache reads need no inference slot. Generation shares the native single-request gate.
        let _guard = if generate {
            Some(
                self.inference
                    .try_lock()
                    .map_err(|_| "One model request is already active. Please wait.")?,
            )
        } else {
            None
        };
        let snapshot = self
            .snapshot
            .read()
            .map_err(|_| "Telemetry unavailable.")?
            .clone();
        let mut output = Vec::new();
        let mut pending = Vec::new();
        let mut seen = HashSet::new();
        for identity in identities {
            let detail = details::inspect(
                &snapshot,
                &identity.workload_id,
                Some((identity.pid, identity.start_time)),
            )?;
            let Some(app) = detail.application else {
                continue;
            };
            let key = profiles::key(&app);
            let profile = self
                .profiles
                .lock()
                .map_err(|_| "Profile cache unavailable.")?
                .get(&key)
                .unwrap_or(detail.profile);
            if profile.source != "inference" {
                output.push(Explanation { identity, profile });
            } else if generate {
                pending.push((identity, key, app));
            }
        }
        if !pending.is_empty() {
            let settings = self
                .settings
                .lock()
                .map_err(|_| "Settings unavailable.")?
                .clone();
            let unique: Vec<_> = pending
                .iter()
                .filter(|(_, key, _)| seen.insert(key.clone()))
                .collect();
            let apps: Vec<_> = unique.iter().map(|(_, _, app)| app.clone()).collect();
            let profiles = crate::providers::purpose(
                "app_explanation",
                None,
                self.engine.app_profiles(&settings, &snapshot, &apps),
            )
            .await?;
            let mut generated = HashMap::new();
            for ((_, key, _), profile) in unique.into_iter().zip(profiles) {
                self.profiles
                    .lock()
                    .map_err(|_| "Profile cache unavailable.")?
                    .insert(key.clone(), profile.clone(), &self.directory)?;
                generated.insert(key.clone(), profile);
            }
            for (identity, key, _) in pending {
                if let Some(profile) = generated.get(&key) {
                    output.push(Explanation {
                        identity,
                        profile: profile.clone(),
                    });
                }
            }
        }
        Ok(output)
    }
}

/// Only names of executable components and fixed role vocabulary cross this boundary.
/// Parent PIDs, private paths, working directories, and arguments are excluded.
pub fn process_context(snapshot: &Snapshot, process: &ProcessRecord) -> serde_json::Value {
    let (chain, _) = details::ancestry(snapshot, process);
    let parents: Vec<_> = chain
        .iter()
        .rev()
        .skip(1)
        .take(4)
        .map(|p| &p.name)
        .collect();
    serde_json::json!({ "observed_parent_names": parents,
        "inferred_role": process.resource_role,
        "disk_interval_seconds": snapshot.interval_ms as f64 / 1000.0,
        "process_age_seconds": snapshot.captured_at.saturating_sub(process.start_time),
        "selected_process_history": observe(snapshot,process),
        "history_scope": "Only samples observed while this identity was inspected. No history before inspection is reconstructed.",
        "limitations": "Parent names and roles are evidence, not proof of a specific task. No arguments, files, transcripts, or network requests were read for this context." })
}

/// Overview uses executable identity, never project labels, task registrations, or paths.
pub fn overview_context(snapshot: &Snapshot) -> serde_json::Value {
    let processes: Vec<_> = snapshot.processes.iter().take(6).map(|p| {
        serde_json::json!({"name":p.name,"cpu_percent":p.cpu_percent,"resident_mib":p.memory_bytes/1048576,
            "inferred_role":p.resource_role,"not_responding":p.unresponsive})
    }).collect();
    let history: Vec<_> = snapshot.history.iter().rev().take(24).rev().collect();
    serde_json::json!({"cpu_percent":snapshot.cpu_percent,"available_memory_mib":snapshot.memory_available_bytes/1048576,
        "total_memory_mib":snapshot.memory_total_bytes/1048576,"pressure":snapshot.pressure.level,
        "thermal":snapshot.platform.thermal,"swap_growth_mib":snapshot.swap_growth_bytes/1048576,
        "busiest_processes":processes,"recent_system_history":history})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_searches_and_sorts_beyond_the_dashboard_and_rejects_bad_queries() {
        let mut s = Snapshot {
            memory_total_bytes: 1 << 30,
            ..Default::default()
        };
        for n in 0..100 {
            let id = format!("group-{n}");
            s.workloads.push(Workload {
                id: id.clone(),
                label: format!("App {n:03}"),
                cpu_percent: n as f32,
                ..Default::default()
            });
            s.processes.push(ProcessRecord {
                pid: n + 100,
                start_time: 2,
                workload_id: id,
                name: format!("worker-{n}"),
                ..Default::default()
            });
        }
        let mut q = Query {
            search: "199".into(),
            view: "processes".into(),
            sort: "cpu".into(),
            filter: "all".into(),
            offset: 0,
        };
        let found = inventory(&s, &q).unwrap();
        assert_eq!(found.rows[0].process.as_ref().unwrap().pid, 199);
        q.search.clear();
        q.view = "apps".into();
        let first = inventory(&s, &q).unwrap();
        assert_eq!(first.total, 100);
        assert_eq!(first.rows.len(), 40);
        assert_eq!(first.rows[0].workload.cpu_percent, 99.0);
        q.offset = 80;
        assert_eq!(inventory(&s, &q).unwrap().rows.len(), 20);
        q.sort = "bad".into();
        assert!(inventory(&s, &q).is_err());
    }
    #[test]
    fn inspected_history_is_bounded_deduplicated_and_separates_reused_pids() {
        let mut entries = VecDeque::new();
        let mut snapshot = Snapshot {
            sample_ready: true,
            captured_at: 1000,
            ..Default::default()
        };
        let mut process = ProcessRecord {
            pid: 123456,
            start_time: 100,
            workload_id: "monitor-history-fixture".into(),
            cpu_percent: 12.0,
            ..Default::default()
        };
        assert_eq!(record(&mut entries, &snapshot, &process).len(), 1);
        assert_eq!(record(&mut entries, &snapshot, &process).len(), 1);
        for _ in 0..40 {
            snapshot.captured_at += 5;
            record(&mut entries, &snapshot, &process);
        }
        let points = record(&mut entries, &snapshot, &process);
        assert_eq!(points.len(), 24);
        process.start_time = 900;
        assert_eq!(record(&mut entries, &snapshot, &process).len(), 1);
    }
    #[test]
    fn model_context_excludes_project_paths_and_reused_parents() {
        let mut s = Snapshot {
            captured_at: 100,
            ..Default::default()
        };
        s.processes = vec![
            ProcessRecord {
                pid: 1,
                start_time: 10,
                name: "Terminal".into(),
                ..Default::default()
            },
            ProcessRecord {
                pid: 2,
                start_time: 20,
                parent_pid: Some(1),
                parent_start_time: Some(10),
                name: "python".into(),
                workload_id: "/private/project".into(),
                working_directory: Some("/private/project".into()),
                ..Default::default()
            },
        ];
        let context = process_context(&s, &s.processes[1]).to_string();
        assert!(context.contains("Terminal"));
        assert!(!context.contains("/private"));
        s.processes[0].start_time = 30;
        assert!(!process_context(&s, &s.processes[1])
            .to_string()
            .contains("Terminal"));
        assert!(!overview_context(&s).to_string().contains("/private"));
    }
}
