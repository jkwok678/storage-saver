//! Instructions and conversation text sent with an advisor question.

use super::types::ChatTurn;
use crate::scan::StorageSnapshot;

pub(super) fn build_system_instructions(snapshot: &StorageSnapshot) -> String {
    let mut ctx = String::with_capacity(4096);
    ctx.push_str(
        "You are the Storage Saver AI Advisor for macOS. Your job is to help the user understand what is taking up disk space on their Mac and identify what they can and should delete.\n\n\
         Rules:\n\
         1. Base your analysis on the scanned storage data below, and use your read-only file/directory tools if you need to inspect specific directories further.\n\
         2. Never attempt to delete, modify, or move files yourself. The user will review and move items to Trash using Storage Saver's UI.\n\
         3. Clearly distinguish between:\n\
            - `safe`: Safe to delete anytime without losing personal data (e.g. ~/Library/Caches/*, Xcode DerivedData, npm/cargo/homebrew caches, old logs, ~/.Trash).\n\
            - `caution`: Worth reviewing before deleting (e.g. large files in ~/Downloads, old .dmg/.zip installers, Docker/VM images, old project node_modules/target build folders, large videos).\n\
            - System/protected paths (/System, /Library, active app bundles) that should NOT be deleted.\n\
         4. Mention concrete file/folder paths and their human-readable sizes in your explanation.\n\
         5. IMPORTANT: Whenever you recommend specific files or folders that the user can delete or review for deletion, include a fenced `recommendations` JSON block at the VERY END of your response in this exact format:\n\
         ```recommendations\n\
         [\n\
           {\n\
             \"path\": \"/Users/example/Library/Caches/com.example.app\",\n\
             \"safety\": \"safe\",\n\
             \"reason\": \"App cache that will be regenerated automatically if needed.\"\n\
           }\n\
         ]\n\
         ```\n\
         Only include real, absolute paths on this Mac in the `recommendations` block.\n\n\
         === SCANNED STORAGE DATA ===\n",
    );

    if let Some(disk) = &snapshot.disk {
        let pct = if disk.total > 0 {
            (disk.used as f64 / disk.total as f64) * 100.0
        } else {
            0.0
        };
        ctx.push_str(&format!(
            "Boot Disk: {:.1}% full — {} used of {} total ({} available)\n\n",
            pct,
            human_bytes(disk.used),
            human_bytes(disk.total),
            human_bytes(disk.available)
        ));
    }

    if let Some(curr) = &snapshot.current_path {
        ctx.push_str(&format!("Currently Viewed Folder: {curr}\n"));
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
