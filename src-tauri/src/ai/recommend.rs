//! Pull recommended paths out of an answer and check them on disk.

use std::collections::HashSet;
use std::env;
use std::path::{Component, Path};

use serde::Deserialize;

use super::types::AiRecommendation;
use crate::scan;

#[derive(Clone, Debug, Deserialize)]
pub(super) struct RawRecommendation {
    pub(super) path: String,
    #[serde(default = "default_safety")]
    pub(super) safety: String,
    #[serde(default)]
    pub(super) reason: String,
}

fn default_safety() -> String {
    "caution".into()
}

pub(super) fn extract_recommendations(raw: &str) -> (String, Vec<RawRecommendation>) {
    for tag in ["```recommendations", "```json-recommendations"] {
        if let Some(start) = raw.find(tag) {
            let before = raw[..start].trim();
            let after_tag = &raw[start + tag.len()..];
            let (json_block, after_fence) = match after_tag.find("```") {
                Some(end) => (&after_tag[..end], after_tag[end + 3..].trim()),
                None => (after_tag, ""),
            };
            let parsed = parse_recommendation_array(json_block);
            let mut clean = before.to_string();
            if !after_fence.is_empty() {
                if !clean.is_empty() {
                    clean.push_str("\n\n");
                }
                clean.push_str(after_fence);
            }
            return (clean, parsed);
        }
    }

    // Fallback: check if there's a trailing ```json block containing path/reason objects
    if let Some(start) = raw.rfind("```json") {
        let after_tag = &raw[start + 7..];
        if let Some(end) = after_tag.find("```") {
            let json_block = &after_tag[..end];
            let parsed = parse_recommendation_array(json_block);
            if !parsed.is_empty() {
                let before = raw[..start].trim().to_string();
                return (before, parsed);
            }
        }
    }

    (raw.trim().to_string(), Vec::new())
}

fn parse_recommendation_array(block: &str) -> Vec<RawRecommendation> {
    let trimmed = block.trim();
    if let Ok(list) = serde_json::from_str::<Vec<RawRecommendation>>(trimmed) {
        return list;
    }
    if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']'))
        && start < end
        && let Ok(list) = serde_json::from_str::<Vec<RawRecommendation>>(&trimmed[start..=end])
    {
        return list;
    }
    Vec::new()
}

pub(super) fn resolve_recommendations(
    raw_recs: Vec<RawRecommendation>,
    cancelled: &dyn Fn() -> bool,
) -> Vec<AiRecommendation> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let home = env::var("HOME").ok();

    for raw in raw_recs {
        if cancelled() {
            break;
        }
        let mut path_str = raw.path.trim().to_string();
        if path_str.starts_with("~/")
            && let Some(ref h) = home
        {
            path_str = format!("{h}/{}", &path_str[2..]);
        }
        if !path_str.starts_with('/') || path_str == "/" {
            continue;
        }
        while path_str.len() > 1 && path_str.ends_with('/') {
            path_str.pop();
        }
        let path = Path::new(&path_str);
        // `..` would point somewhere other than the path shown, and protected folders cannot be
        // trashed anyway, so neither is worth offering.
        if path.components().any(|c| c == Component::ParentDir) || scan::is_protected_path(path) {
            continue;
        }
        if !seen.insert(path_str.clone()) {
            continue;
        }
        let path = Path::new(&path_str);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path_str.clone());
        let inspected = scan::inspect_path_size(path);
        let (bytes, is_dir, exists) = match inspected {
            Some((b, d)) => (Some(b), d, true),
            None => (None, false, false),
        };
        let safety = if raw.safety.eq_ignore_ascii_case("safe") {
            "safe".into()
        } else {
            "caution".into()
        };
        out.push(AiRecommendation {
            path: path_str,
            name,
            is_dir,
            bytes,
            safety,
            reason: raw.reason.trim().to_string(),
            exists,
        });
    }
    // Sort existing items with largest byte sizes first
    out.sort_by(|a, b| {
        b.exists
            .cmp(&a.exists)
            .then_with(|| b.bytes.unwrap_or(0).cmp(&a.bytes.unwrap_or(0)))
    });
    out
}
