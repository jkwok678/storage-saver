//! Instructions and conversation text sent with an advisor question.

use super::types::ChatTurn;
use crate::platform::ADVISOR_RULES;
use crate::scan::StorageSnapshot;

pub(super) fn build_system_instructions(snapshot: &StorageSnapshot) -> String {
    let mut ctx = String::with_capacity(4096);
    ctx.push_str(ADVISOR_RULES);

    for (label, disk) in &snapshot.volumes {
        let pct = if disk.total > 0 {
            (disk.used as f64 / disk.total as f64) * 100.0
        } else {
            0.0
        };
        ctx.push_str(&format!(
            "{label}: {:.1}% full — {} used of {} total ({} available)\n",
            pct,
            human_bytes(disk.used),
            human_bytes(disk.total),
            human_bytes(disk.available)
        ));
    }
    if !snapshot.volumes.is_empty() {
        ctx.push('\n');
    }

    if let Some(curr) = &snapshot.current_path {
        let shown = if curr.is_empty() { "This PC" } else { curr };
        ctx.push_str(&format!("Currently Viewed Folder: {shown}\n"));
        if !snapshot.current_items.is_empty() {
            ctx.push_str("Items in Current Folder (sorted by size):\n");
            for item in &snapshot.current_items {
                let kind = if item.is_dir { "dir" } else { "file" };
                ctx.push_str(&format!(
                    "  - {} ({}, {}): {}\n",
                    item.path,
                    kind,
                    human_bytes(item.bytes),
                    item.name
                ));
            }
            ctx.push('\n');
        }
    }

    if !snapshot.notable_locations.is_empty() {
        ctx.push_str("Known User Cache & Cleanup Locations:\n");
        for item in &snapshot.notable_locations {
            ctx.push_str(&format!(
                "  - {} ({}): {}\n",
                item.path,
                human_bytes(item.bytes),
                item.name
            ));
        }
        ctx.push('\n');
    }

    if !snapshot.top_folders.is_empty() {
        ctx.push_str("Largest Scanned Folders (total size including subfolders):\n");
        for item in &snapshot.top_folders {
            ctx.push_str(&format!(
                "  - {} (total: {}, direct files: {})\n",
                item.path,
                human_bytes(item.bytes),
                human_bytes(item.own_bytes)
            ));
        }
        ctx.push('\n');
    }

    if !snapshot.dense_folders.is_empty() {
        ctx.push_str("Folders with Largest Direct File Usage (own bytes):\n");
        for item in &snapshot.dense_folders {
            ctx.push_str(&format!(
                "  - {} (direct files: {}, total: {})\n",
                item.path,
                human_bytes(item.own_bytes),
                human_bytes(item.bytes)
            ));
        }
        ctx.push('\n');
    }

    if !snapshot.largest_files.is_empty() {
        ctx.push_str("Largest Individual Files Found:\n");
        for file in &snapshot.largest_files {
            ctx.push_str(&format!(
                "  - {} ({})\n",
                file.path,
                human_bytes(file.bytes)
            ));
        }
        ctx.push('\n');
    }

    ctx
}

pub(super) fn build_conversation_prompt(user_prompt: &str, history: &[ChatTurn]) -> String {
    if history.is_empty() {
        return user_prompt.to_string();
    }
    let mut out = String::from("Conversation so far:\n");
    for turn in history.iter().rev().take(8).rev() {
        let speaker = if turn.role == "assistant" {
            "Assistant"
        } else {
            "User"
        };
        out.push_str(&format!("{speaker}: {}\n\n", turn.content));
    }
    out.push_str(&format!("User's new question:\n{user_prompt}"));
    out
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut val = bytes as f64;
    let mut unit = 0;
    while val >= 1024.0 && unit < UNITS.len() - 1 {
        val /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{val:.1} {}", UNITS[unit])
    }
}
