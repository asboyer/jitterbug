//! Plain-English task narratives from bounded harness context and measured resource roles.
use crate::workflows::{AgentWorkflow, RoleUsage};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    hash::{DefaultHasher, Hash, Hasher},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Assignment {
    pub agent: String,
    pub request: String,
    pub source: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Narrative {
    pub assignments: Vec<Assignment>,
    pub task_key: String,
    pub activity: String,
    pub resources: String,
    pub tip: String,
    pub tip_basis: String,
}

pub fn current_roles(workflow: &AgentWorkflow) -> Vec<RoleUsage> {
    let mut roles: BTreeMap<String, RoleUsage> = BTreeMap::new();
    for role in workflow.agents.iter().flat_map(|s| &s.roles) {
        let usage = roles.entry(role.role.clone()).or_insert_with(|| RoleUsage {
            role: role.role.clone(),
            ..Default::default()
        });
        usage.cpu_percent += role.cpu_percent;
        usage.memory_bytes = usage.memory_bytes.saturating_add(role.memory_bytes);
        usage.process_count += role.process_count;
    }
    roles.into_values().collect()
}

pub fn describe(workflow: &AgentWorkflow) -> Narrative {
    let mut sessions: Vec<_> = workflow
        .agents
        .iter()
        .filter(|s| !s.context.request.is_empty())
        .collect();
    sessions.sort_by(|a, b| {
        b.context
            .updated_at
            .cmp(&a.context.updated_at)
            .then_with(|| a.run_id.cmp(&b.run_id))
    });
    let assignments: Vec<_> = sessions
        .iter()
        .take(4)
        .map(|s| Assignment {
            agent: s.agent.clone(),
            request: if s.context.latest_request.is_empty() {
                s.context.request.clone()
            } else {
                format!(
                    "Initial request: {} Follow-up: {}",
                    s.context.request, s.context.latest_request
                )
            },
            source: s.context.source.clone(),
        })
        .collect();
    let mut hash = DefaultHasher::new();
    // No metrics in this key: only the task-purpose rewrite is cached, never the diagnosis.
    let requests: Vec<_> = assignments.iter().map(|a| a.request.as_str()).collect();
    (1u32, requests).hash(&mut hash);
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    for (role, count) in workflow
        .agents
        .iter()
        .flat_map(|s| &s.context.completed_tools)
    {
        let total = counts.entry(role).or_default();
        *total = total.saturating_add(*count);
    }
    let activity = if counts.is_empty() {
        if assignments.is_empty() {
            "Task intent is unavailable for this launch. Connect harness hooks or supply a task description for future runs.".into()
        } else {
            "The request describes the intended work. No completed tool activity has been reported yet.".into()
        }
    } else {
        let descriptions: Vec<_> = counts
            .iter()
            .map(|(role, count)| {
                format!(
                    "{count} {}{}",
                    match *role {
                        "read" => "file read/search",
                        "edit" => "file edit call",
                        "command" => "shell command call",
                        _ => "other tool call",
                    },
                    if *count == 1 { "" } else { "s" }
                )
            })
            .collect();
        format!("The harness reported {}. These are tool events, not proof that the requested feature is complete.", descriptions.join(", "))
    };
    let resources = if workflow.observed_samples == 0 {
        "No resource samples were captured for this task in this app session, so its resource use cannot be explained yet.".into()
    } else {
        let live = workflow.status == "active";
        let mut roles = if live {
            current_roles(workflow)
        } else {
            workflow.peak_roles.clone()
        };
        roles.sort_by(|a, b| {
            b.cpu_percent
                .total_cmp(&a.cpu_percent)
                .then_with(|| b.memory_bytes.cmp(&a.memory_bytes))
        });
        let known = roles
            .iter()
            .find(|r| r.process_count > 0 && r.role != "other");
        let cpu = if live {
            workflow.cpu_percent
        } else {
            workflow.peak_cpu_percent
        };
        let prefix = if live {
            "In the latest sample"
        } else {
            "In the busiest observed CPU sample"
        };
        let detail = known.map(|role| {
            let purpose = match role.role.as_str() {
                "test" => "Test runners execute checks and hold their workers and fixtures in memory",
                "build" => "Build tools compile and bundle the project",
                "typecheck" => "Type checkers load and analyze the project's types",
                "lint" => "Linters inspect source files for code issues",
                "agent" => "The agent harness coordinates model requests and local tools; this does not establish where model inference runs",
                _ => "These are unclassified local tools",
            };
            format!(" {purpose}. The {} processes accounted for {} of total CPU capacity and {} MiB resident memory in that sample.", role.role, cpu_label(role.cpu_percent), role.memory_bytes / 1048576)
        }).unwrap_or_else(|| " The observed processes have no recognized tool role, so the specific activity is unknown.".into());
        if cpu == 0.0 {
            "No CPU activity registered in the captured samples. Short-lived work can fall between samples, so this does not mean the task did no work. Resident memory can remain allocated while processes wait.".into()
        } else {
            format!("{prefix}, this task used {} of total CPU capacity.{detail} This locates resource use; it does not identify a code hotspot or prove waste.", cpu_label(cpu))
        }
    };
    let recommendation = workflow.suggestions.iter().max_by_key(|s| {
        (
            s.current,
            match s.id.as_str() {
                "memory-headroom" => 4,
                id if id.starts_with("overlapping-") => 3,
                "worker-amplification" => 2,
                _ => 1,
            },
        )
    });
    let (tip, tip_basis) = recommendation.map_or_else(|| (
        "No resource optimization is justified by the samples observed so far. Keep measuring during the slow part of the task.".into(),
        "Local rules · no action applied".into()), |s| (s.recommendation.clone(), format!("{} · {} · advisory", s.title, if s.current { "observed now" } else { "observed earlier in this app session" })));
    Narrative {
        assignments,
        task_key: format!("task-{:x}", hash.finish()),
        activity,
        resources,
        tip,
        tip_basis,
    }
}

fn cpu_label(value: f32) -> String {
    if value > 0.0 && value < 0.1 {
        "<0.1%".into()
    } else {
        format!("{value:.1}%")
    }
}

/// Explicit task-summary requests use only the excerpts previewed in the UI. Normal chat and
/// automatic investigations continue to receive anonymous metrics, never this payload.
pub fn model_payload(workflow: &AgentWorkflow) -> serde_json::Value {
    serde_json::json!({"task_requests": workflow.narrative.assignments.iter().map(|a| &a.request).collect::<Vec<_>>()})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{harness::Context, workflows::AgentSession};
    #[test]
    fn unsampled_zero_and_tiny_cpu_have_distinct_explanations() {
        let mut workflow = AgentWorkflow {
            status: "completed".into(),
            ..Default::default()
        };
        assert!(describe(&workflow)
            .resources
            .contains("No resource samples"));
        workflow.observed_samples = 3;
        assert!(describe(&workflow)
            .resources
            .contains("No CPU activity registered"));
        assert!(describe(&workflow).resources.contains("between samples"));
        workflow.peak_cpu_percent = 0.025;
        assert!(describe(&workflow).resources.contains("<0.1%"));
        assert!(!describe(&workflow).resources.contains("used 0.0%"));
    }

    #[test]
    fn purpose_cache_changes_with_request_not_metrics_and_excludes_identity() {
        let mut w = AgentWorkflow {
            task_id: "private-task".into(),
            agents: vec![AgentSession {
                agent: "private-agent".into(),
                worktree: "/private/repo".into(),
                context: Context::supplied("Build a search screen"),
                ..Default::default()
            }],
            ..Default::default()
        };
        w.narrative = describe(&w);
        let key = w.narrative.task_key.clone();
        w.cpu_percent = 35.;
        assert_eq!(describe(&w).task_key, key);
        let payload = model_payload(&w).to_string();
        assert!(payload.contains("Build a search screen"));
        for private in ["private-task", "private-agent", "/private/repo"] {
            assert!(!payload.contains(private));
        }
        w.agents[0].context.request = "Add search tests".into();
        assert_ne!(describe(&w).task_key, key);
    }
    #[test]
    fn completed_narrative_uses_roles_from_same_peak_sample_not_current_zeroes() {
        let w = AgentWorkflow {
            status: "completed".into(),
            observed_samples: 2,
            peak_cpu_percent: 15.,
            peak_roles: vec![RoleUsage {
                role: "test".into(),
                process_count: 2,
                cpu_percent: 12.,
                memory_bytes: 128 << 20,
            }],
            ..Default::default()
        };
        let n = describe(&w);
        assert!(n.resources.contains("12.0%"));
        assert!(n.resources.contains("128 MiB"));
        assert!(n.resources.contains("busiest observed"));
        assert!(!n.resources.contains("latest sample"));
        assert!(describe(&AgentWorkflow::default())
            .resources
            .contains("No resource samples"));
    }
}
