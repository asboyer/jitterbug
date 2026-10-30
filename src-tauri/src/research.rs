//! Bounded official-source troubleshooting; no tab URLs or prompts leave the device.
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

const URL: &str = "https://support.apple.com/en-us/102564";
const GUIDANCE: &str = "Apple recommends testing optional Safari extensions and trying the affected page in a private window to isolate website data. Page-loading failures can also involve network settings, VPNs, or Private Relay. Save work before quitting; force quit is a last resort for an app that will not quit normally.";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Research {
    pub title: String,
    pub url: String,
    pub summary: String,
    pub checked_at: Option<u64>,
    pub status: String,
}

pub fn slowdown(question: &str) -> bool {
    let q = question.to_lowercase();
    q.split(|c: char| !c.is_alphabetic()).any(|word| {
        [
            "slow",
            "lag",
            "freez",
            "froze",
            "hang",
            "stuck",
            "unresponsive",
        ]
        .iter()
        .any(|prefix| word.starts_with(prefix))
    }) || [
        "not responding",
        "won't load",
        "not working",
        "one tab",
        "whole window",
    ]
    .iter()
    .any(|term| q.contains(term))
}

pub async fn safari(client: &reqwest::Client, directory: &Path, online: bool) -> Research {
    let path = directory.join("safari-guidance.json");
    if let Ok(Some(mut saved)) = crate::storage::read::<Option<Research>>(&path) {
        if saved.url == URL
            && saved.summary == GUIDANCE
            && saved
                .checked_at
                .is_some_and(|at| at <= crate::types::now() && crate::types::now() - at < 86400)
        {
            saved.status = "cached".into();
            return saved;
        }
    }
    let mut result = Research {
        title: "Apple: If Safari doesn't work as expected".into(),
        url: URL.into(),
        summary: GUIDANCE.into(),
        checked_at: None,
        status: if online {
            "lookup_unavailable"
        } else {
            "offline_reference"
        }
        .into(),
    };
    if online
        && tokio::time::timeout(Duration::from_secs(5), verify_article(client))
            .await
            .is_ok_and(|v| v)
    {
        result.checked_at = Some(crate::types::now());
        result.status = "checked".into();
        let _ = crate::storage::write(&path, &Some(&result));
    }
    result
}

async fn verify_article(client: &reqwest::Client) -> bool {
    let Ok(mut response) = client.get(URL).timeout(Duration::from_secs(4)).send().await else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > 512 * 1024 {
                    return false;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(_) => return false,
        }
    }
    let html = String::from_utf8_lossy(&body);
    // Verify the reference sections, without sending arbitrary HTML to the model.
    html.contains("Check Safari extensions")
        && html.contains("private window")
        && html.contains("network settings")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn offline_reference_does_not_claim_a_fresh_lookup() {
        let directory = std::env::temp_dir().join("jitterbug-offline-research-fixture");
        let result = safari(&reqwest::Client::new(), &directory, false).await;
        assert_eq!(result.status, "offline_reference");
        assert!(result.checked_at.is_none());
        assert_eq!(result.url, URL);
        assert!(slowdown("why is Safari lagging"));
        assert!(!slowdown("what is CPU"));
        assert!(!slowdown("change the priority and flag the result"));
    }
}
