//! Windows path rules. Pure string work, so it runs and is tested on any OS.

use super::FileKey;

const VERBATIM: &str = r"\\?\";

/// Seconds from 1601-01-01, where FILETIME starts, to the Unix epoch.
const FILETIME_UNIX_GAP: i64 = 11_644_473_600;

/// Folders and files that are never offered for the Recycle Bin, as lowercase text.
pub(crate) struct ProtectedEnv {
    pub(crate) system_root: String,
    pub(crate) program_files: String,
    pub(crate) program_files_x86: String,
    pub(crate) program_data: String,
    pub(crate) profile: String,
}

/// Folders of the user profile that are protected themselves, though not what is inside them.
const PROFILE_FOLDERS: &[&str] = &[
    "desktop",
    "documents",
    "downloads",
    "pictures",
    "music",
    "videos",
    "appdata",
    "appdata\\local",
    "appdata\\roaming",
    "appdata\\locallow",
    "onedrive",
];

/// Names at a drive root that belong to Windows.
const DRIVE_ROOT_ENTRIES: &[&str] = &[
    "$recycle.bin",
    "pagefile.sys",
    "hiberfil.sys",
    "swapfile.sys",
    "users",
    "users\\public",
];

/// The plain form of a path: `X:\folder\file`, upper-case drive letter, `\` separators, no
/// trailing `\` except at a drive root. `None` for anything else, such as network paths,
/// drive-relative paths, `..` and control characters.
pub(crate) fn normalize(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.chars().any(|c| c < ' ') {
        return None;
    }
    let slashed = trimmed.replace('/', "\\");
    let path = match slashed.strip_prefix(VERBATIM) {
        Some(rest) => {
            if rest
                .get(..4)
                .is_some_and(|head| head.eq_ignore_ascii_case("UNC\\"))
            {
                return None;
            }
            rest
        }
        None => slashed.as_str(),
    };

    let bytes = path.as_bytes();
    if bytes.len() < 2 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' {
        return None;
    }
    let tail = &path[2..];
    if !tail.is_empty() && !tail.starts_with('\\') {
        return None;
    }
    let mut out = String::with_capacity(path.len());
    out.push(char::from(bytes[0].to_ascii_uppercase()));
    out.push_str(":\\");
    let mut first = true;
    for part in tail.split('\\') {
        match part {
            "" | "." => continue,
            ".." => return None,
            _ => {}
        }
        if !first {
            out.push('\\');
        }
        out.push_str(part);
        first = false;
    }
    Some(out)
}

/// `path` in the `\\?\` form, which the system calls take without the 260 character limit.
/// Expects a path from `normalize`.
pub(crate) fn verbatim(path: &str) -> String {
    if path.starts_with(VERBATIM) {
        path.to_owned()
    } else {
        format!("{VERBATIM}{path}")
    }
}

pub(crate) fn is_drive_root(path: &str) -> bool {
    normalize(path).is_some_and(|plain| plain.len() == 3)
}

/// The drive letter as a number, `A` being 0.
pub(crate) fn drive_index(path: &str) -> Option<u8> {
    normalize(path).map(|plain| plain.as_bytes()[0] - b'A')
}

/// Folds the volume serial and a 128-bit file id into the key that names a file on any volume.
/// A 64-bit id fills only the low half, so the volume serial stays readable in the key.
pub(crate) fn fold_id(serial: u32, id: [u8; 16]) -> FileKey {
    let id = u128::from_le_bytes(id);
    (u64::from(serial) ^ (id >> 64) as u64, id as u64)
}

/// Seconds since the Unix epoch for a FILETIME (100 ns units since 1601).
pub(crate) fn filetime_to_unix(filetime: i64) -> Option<u64> {
    u64::try_from(filetime / 10_000_000 - FILETIME_UNIX_GAP).ok()
}

fn within(path: &str, base: &str) -> bool {
    !base.is_empty()
        && path
            .strip_prefix(base)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('\\'))
}

/// Whether `path` is one Storage Saver must not move: drive roots, Windows and program
/// folders, the profile and its main folders, and the system files at a drive root.
/// `path` must be lowercase and from `normalize`.
pub(crate) fn is_protected(path: &str, env: &ProtectedEnv) -> bool {
    let Some(rest) = path
        .get(1..3)
        .filter(|head| *head == ":\\")
        .and_then(|_| path.get(3..))
    else {
        return false;
    };
    if rest.is_empty()
        || DRIVE_ROOT_ENTRIES.contains(&rest)
        || within(rest, "system volume information")
    {
        return true;
    }
    if [&env.system_root, &env.program_files, &env.program_files_x86]
        .into_iter()
        .any(|base| within(path, base))
    {
        return true;
    }
    if !env.program_data.is_empty() && path == env.program_data {
        return true;
    }
    path.strip_prefix(env.profile.as_str())
        .filter(|_| !env.profile.is_empty())
        .is_some_and(|below| {
            below.is_empty()
                || below
                    .strip_prefix('\\')
                    .is_some_and(|folder| PROFILE_FOLDERS.contains(&folder))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_accepts_and_cleans_drive_paths() {
        let cases = [
            ("C:\\", "C:\\"),
            ("c:", "C:\\"),
            ("c:/", "C:\\"),
            ("  d:\\Users\\Me  ", "D:\\Users\\Me"),
            ("c:/windows/system32/", "C:\\windows\\system32"),
            ("C:\\a\\\\b\\.\\c", "C:\\a\\b\\c"),
            ("\\\\?\\C:\\Windows", "C:\\Windows"),
            ("//?/c:/Windows/", "C:\\Windows"),
            ("\\\\?\\C:", "C:\\"),
            (
                "C:\\caf\u{e9}\\\u{6587}\u{4ef6}.txt",
                "C:\\caf\u{e9}\\\u{6587}\u{4ef6}.txt",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize(input).as_deref(), Some(expected), "{input:?}");
        }
    }

    #[test]
    fn normalize_rejects_everything_else() {
        let cases = [
            "",
            "   ",
            "\\\\?\\UNC\\server\\share",
            "\\\\?\\unc\\server\\share\\x",
            "\\\\server\\share\\x",
            "\\\\.\\C:\\x",
            "\\\\?\\Volume{1234}\\x",
            "C:foo",
            "C:foo\\bar",
            "\\foo",
            "/foo",
            "foo\\bar",
            "1:\\x",
            "C:\\a\\..\\b",
            "C:\\..",
            "C:\\a\nb",
            "C:\\a\u{0}b",
        ];
        for input in cases {
            assert_eq!(normalize(input), None, "{input:?}");
        }
    }

    #[test]
    fn verbatim_adds_the_prefix_once() {
        assert_eq!(verbatim("C:\\a\\b"), "\\\\?\\C:\\a\\b");
        assert_eq!(verbatim("\\\\?\\C:\\a"), "\\\\?\\C:\\a");
    }

    #[test]
    fn drive_roots_and_indexes() {
        assert!(is_drive_root("C:\\"));
        assert!(is_drive_root("d:"));
        assert!(is_drive_root("\\\\?\\E:\\"));
        assert!(!is_drive_root("C:\\Windows"));
        assert!(!is_drive_root(""));
        assert!(!is_drive_root("\\\\server\\share"));
        assert_eq!(drive_index("C:\\x"), Some(2));
        assert_eq!(drive_index("a:"), Some(0));
        assert_eq!(drive_index("z:/"), Some(25));
        assert_eq!(drive_index(""), None);
        assert_eq!(drive_index("\\\\server\\share"), None);
    }

    #[test]
    fn file_ids_fold_with_the_volume_serial() {
        let mut id = [0u8; 16];
        id[..8].copy_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
        assert_eq!(fold_id(7, id), (7, 0x0102_0304_0506_0708));
        id[8..].copy_from_slice(&0x10u64.to_le_bytes());
        assert_eq!(fold_id(7, id), (7 ^ 0x10, 0x0102_0304_0506_0708));
        assert_ne!(fold_id(1, id), fold_id(2, id));
    }

    #[test]
    fn filetimes_become_unix_seconds() {
        assert_eq!(filetime_to_unix(116_444_736_000_000_000), Some(0));
        assert_eq!(filetime_to_unix(116_444_736_010_000_000), Some(1));
        assert_eq!(filetime_to_unix(0), None);
    }

    fn env() -> ProtectedEnv {
        ProtectedEnv {
            system_root: "c:\\windows".into(),
            program_files: "c:\\program files".into(),
            program_files_x86: "c:\\program files (x86)".into(),
            program_data: "c:\\programdata".into(),
            profile: "c:\\users\\me".into(),
        }
    }

    #[test]
    fn protected_paths() {
        let protected = [
            "c:\\",
            "d:\\",
            "c:\\windows",
            "c:\\windows\\system32",
            "c:\\windows\\system32\\drivers\\etc",
            "c:\\program files",
            "c:\\program files\\app",
            "c:\\program files (x86)",
            "c:\\programdata",
            "c:\\users",
            "d:\\users",
            "c:\\users\\public",
            "c:\\users\\me",
            "c:\\users\\me\\desktop",
            "c:\\users\\me\\documents",
            "c:\\users\\me\\downloads",
            "c:\\users\\me\\pictures",
            "c:\\users\\me\\music",
            "c:\\users\\me\\videos",
            "c:\\users\\me\\appdata",
            "c:\\users\\me\\appdata\\local",
            "c:\\users\\me\\appdata\\roaming",
            "c:\\users\\me\\appdata\\locallow",
            "c:\\users\\me\\onedrive",
            "c:\\$recycle.bin",
            "e:\\$recycle.bin",
            "c:\\system volume information",
            "c:\\system volume information\\tracking.log",
            "c:\\pagefile.sys",
            "d:\\hiberfil.sys",
            "c:\\swapfile.sys",
        ];
        for path in protected {
            assert!(is_protected(path, &env()), "{path}");
        }
    }

    #[test]
    fn ordinary_paths_are_not_protected() {
        let ordinary = [
            "c:\\windows2",
            "c:\\program files2",
            "c:\\programdata\\cache",
            "c:\\users\\me\\appdata\\local\\temp",
            "c:\\users\\me\\appdata\\local\\temp\\x",
            "c:\\users\\me\\downloads\\setup.exe",
            "c:\\users\\me2",
            "c:\\users\\other",
            "c:\\users\\public\\documents",
            "c:\\data",
            "d:\\windows",
            "d:\\projects\\pagefile.sys",
            "c:\\$recycle.bin\\s-1-5-21\\x",
            "relative",
            "",
        ];
        for path in ordinary {
            assert!(!is_protected(path, &env()), "{path}");
        }
    }

    #[test]
    fn missing_environment_protects_nothing_extra() {
        let empty = ProtectedEnv {
            system_root: String::new(),
            program_files: String::new(),
            program_files_x86: String::new(),
            program_data: String::new(),
            profile: String::new(),
        };
        assert!(is_protected("c:\\", &empty));
        assert!(!is_protected("c:\\windows", &empty));
        assert!(!is_protected("c:\\users\\me\\desktop", &empty));
        assert!(!is_protected("c:\\anything\\else", &empty));
    }
}
