//! Directory reads through `GetFileInformationByHandleEx`, so a folder's files need no open of their own.

use std::cell::RefCell;
use std::ffi::c_void;
use std::io::{self, ErrorKind};
use std::mem::{offset_of, size_of};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{
    ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES, ERROR_NOT_SUPPORTED, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_RECALL_ON_OPEN,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_FULL_DIR_INFO, FILE_ID_BOTH_DIR_INFO, FILE_ID_EXTD_DIR_INFO, FILE_INFO_BY_HANDLE_CLASS,
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FILE_STANDARD_INFO, FileFullDirectoryInfo, FileIdBothDirectoryInfo,
    FileIdExtdDirectoryInfo, FileStandardInfo, GetFileInformationByHandleEx,
    GetVolumeInformationByHandleW, OPEN_EXISTING,
};

use crate::platform::{BulkEntry, winpath};

/// Set in a reparse tag when the point names another path: symlinks, junctions and mount points.
const NAME_SURROGATE: u32 = 0x2000_0000;

thread_local! {
    /// Reused across reads. `u64` keeps it 8-byte aligned.
    static BULK_BUFFER: RefCell<Vec<u64>> = RefCell::new(vec![0; 64 * 1024 / 8]);
}

/// Volume serial numbers by drive letter, read once. Every folder a walk can reach below a
/// drive letter is on that volume, because mount points are not followed.
static SERIALS: [OnceLock<u32>; 26] = [const { OnceLock::new() }; 26];

/// Where each field sits in one of the directory information structures.
struct Layout {
    class: FILE_INFO_BY_HANDLE_CLASS,
    /// Holds the reparse tag. The classes without a tag field keep it in `EaSize`.
    tag_at: usize,
    id: IdField,
    name_at: usize,
}

enum IdField {
    None,
    /// A 64-bit file id.
    Short(usize),
    /// A 128-bit file id.
    Wide(usize),
}

/// Newest first. An older file system or Windows may refuse a class, so reads fall back.
const LAYOUTS: [Layout; 3] = [
    Layout {
        class: FileIdExtdDirectoryInfo,
        tag_at: offset_of!(FILE_ID_EXTD_DIR_INFO, ReparsePointTag),
        id: IdField::Wide(offset_of!(FILE_ID_EXTD_DIR_INFO, FileId)),
        name_at: offset_of!(FILE_ID_EXTD_DIR_INFO, FileName),
    },
    Layout {
        class: FileIdBothDirectoryInfo,
        tag_at: offset_of!(FILE_ID_BOTH_DIR_INFO, EaSize),
        id: IdField::Short(offset_of!(FILE_ID_BOTH_DIR_INFO, FileId)),
        name_at: offset_of!(FILE_ID_BOTH_DIR_INFO, FileName),
    },
    Layout {
        class: FileFullDirectoryInfo,
        tag_at: offset_of!(FILE_FULL_DIR_INFO, EaSize),
        id: IdField::None,
        name_at: offset_of!(FILE_FULL_DIR_INFO, FileName),
    },
];

// The leading fields are the same in all three structures.
const NEXT_AT: usize = offset_of!(FILE_FULL_DIR_INFO, NextEntryOffset);
const WRITTEN_AT: usize = offset_of!(FILE_FULL_DIR_INFO, LastWriteTime);
const ALLOCATED_AT: usize = offset_of!(FILE_FULL_DIR_INFO, AllocationSize);
const ATTRIBUTES_AT: usize = offset_of!(FILE_FULL_DIR_INFO, FileAttributes);
const NAME_LENGTH_AT: usize = offset_of!(FILE_FULL_DIR_INFO, FileNameLength);

/// One directory entry as the file system reports it.
pub(super) struct RawEntry {
    pub(super) name: String,
    pub(super) attributes: u32,
    pub(super) reparse_tag: u32,
    pub(super) allocated: u64,
    /// FILETIME of the last write.
    pub(super) written: i64,
    /// Zero when the file system gave no id.
    pub(super) id: [u8; 16],
}

impl RawEntry {
    pub(super) fn is_dir(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_DIRECTORY != 0
    }

    /// Symlinks, junctions and mount points. They are not followed.
    pub(super) fn is_link(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            && self.reparse_tag & NAME_SURROGATE != 0
    }

    /// Opening it would download its contents, so it is not entered.
    pub(super) fn is_recall_on_open(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_RECALL_ON_OPEN != 0
    }
}

/// The volume of the folder being read, for keys that name a file across volumes.
pub(super) struct VolumeSerial {
    handle: HANDLE,
    drive: Option<u8>,
    read: Option<Option<u32>>,
}

impl VolumeSerial {
    pub(super) fn get(&mut self) -> Option<u32> {
        *self.read.get_or_insert_with(|| {
            if let Some(serial) = self
                .drive
                .and_then(|drive| SERIALS[usize::from(drive)].get())
            {
                return Some(*serial);
            }
            let mut serial = 0u32;
            // SAFETY: the handle is open for the whole read, and only the serial out-pointer is given.
            let ok = unsafe {
                GetVolumeInformationByHandleW(
                    self.handle,
                    ptr::null_mut(),
                    0,
                    &mut serial,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    0,
                )
            };
            if ok == 0 {
                return None;
            }
            if let Some(drive) = self.drive {
                let _ = SERIALS[usize::from(drive)].set(serial);
            }
            Some(serial)
        })
    }
}

/// `path` as the null-terminated UTF-16 the system calls take, in the `\\?\` form.
pub(super) fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    let plain = path
        .to_str()
        .and_then(winpath::normalize)
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "unsupported path"))?;
    Ok(winpath::verbatim(&plain)
        .encode_utf16()
        .chain(Some(0))
        .collect())
}

/// Opens a file or folder without following a link at the end of the path.
pub(super) fn open(path: &Path, access: u32) -> io::Result<OwnedHandle> {
    let wide = wide_path(path)?;
    // SAFETY: `wide` is null-terminated and outlives the call. The other pointers are null.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateFileW returned a new handle that nothing else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// The space the file takes on disk, from the file system itself. `None` when it cannot be opened.
pub(super) fn allocated_size(path: &Path) -> Option<u64> {
    let file = open(path, FILE_READ_ATTRIBUTES).ok()?;
    let mut info = FILE_STANDARD_INFO::default();
    // SAFETY: `info` is the structure FileStandardInfo asks for, and its size is passed.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileStandardInfo,
            (&raw mut info).cast::<c_void>(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    };
    (ok != 0).then(|| u64::try_from(info.AllocationSize).unwrap_or(0))
}

/// Calls `each` with every entry of `dir` except `.` and `..`.
pub(super) fn enumerate(
    dir: &Path,
    mut each: impl FnMut(&RawEntry, &mut VolumeSerial),
) -> io::Result<()> {
    let folder = open(dir, FILE_LIST_DIRECTORY)?;
    let handle = folder.as_raw_handle();
    let mut serial = VolumeSerial {
        handle,
        drive: dir.to_str().and_then(winpath::drive_index),
        read: None,
    };

    BULK_BUFFER.with_borrow_mut(|buffer| {
        let size = buffer.len() * 8;
        let mut layout = 0;
        let mut first = true;
        loop {
            // SAFETY: the buffer is `size` bytes, and the class is one of the three read below.
            let ok = unsafe {
                GetFileInformationByHandleEx(
                    handle,
                    LAYOUTS[layout].class,
                    buffer.as_mut_ptr().cast::<c_void>(),
                    size as u32,
                )
            };
            if ok == 0 {
                let err = io::Error::last_os_error();
                let code = err.raw_os_error().and_then(|code| u32::try_from(code).ok());
                if code == Some(ERROR_NO_MORE_FILES) {
                    return Ok(());
                }
                let refused = matches!(code, Some(ERROR_INVALID_PARAMETER | ERROR_NOT_SUPPORTED));
                if first && refused && layout + 1 < LAYOUTS.len() {
                    layout += 1;
                    continue;
                }
                return Err(err);
            }
            first = false;
            // SAFETY: the buffer is `size` initialized bytes, and u8 has no alignment needs.
            let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), size) };
            let mut offset = 0;
            loop {
                let Some(entry) = bytes.get(offset..) else {
                    return Err(malformed());
                };
                let next = read_u32(entry, NEXT_AT).ok_or_else(malformed)? as usize;
                if let Some(raw) = parse_entry(entry, &LAYOUTS[layout]).ok_or_else(malformed)? {
                    each(&raw, &mut serial);
                }
                if next == 0 {
                    break;
                }
                offset += next;
            }
        }
    })
}

fn malformed() -> io::Error {
    io::Error::new(
        ErrorKind::InvalidData,
        "the file system returned a malformed directory entry",
    )
}

/// `Ok(None)` for `.` and `..`, `None` for an entry that does not fit the buffer.
fn parse_entry(entry: &[u8], layout: &Layout) -> Option<Option<RawEntry>> {
    let length = read_u32(entry, NAME_LENGTH_AT)? as usize;
    let name_bytes = entry.get(layout.name_at..layout.name_at.checked_add(length)?)?;
    let units: Vec<u16> = name_bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    if units == [u16::from(b'.')] || units == [u16::from(b'.'), u16::from(b'.')] {
        return Some(None);
    }

    let attributes = read_u32(entry, ATTRIBUTES_AT)?;
    let reparse_tag = if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        read_u32(entry, layout.tag_at)?
    } else {
        0
    };
    let mut id = [0u8; 16];
    match layout.id {
        IdField::None => {}
        IdField::Short(at) => id[..8].copy_from_slice(entry.get(at..at + 8)?),
        IdField::Wide(at) => id.copy_from_slice(entry.get(at..at + 16)?),
    }
    Some(Some(RawEntry {
        name: String::from_utf16_lossy(&units),
        attributes,
        reparse_tag,
        allocated: u64::try_from(read_i64(entry, ALLOCATED_AT)?).unwrap_or(0),
        written: read_i64(entry, WRITTEN_AT)?,
        id,
    }))
}

/// List `dir`. One call returns the type, allocated size and file id of a whole batch of
/// entries, so files cost no open of their own.
pub(crate) fn read_dir_bulk(dir: &Path, mut each: impl FnMut(BulkEntry)) -> io::Result<()> {
    enumerate(dir, |entry, serial| {
        each(if entry.is_link() {
            BulkEntry::Other
        } else if entry.is_dir() {
            if entry.is_recall_on_open() {
                BulkEntry::Other
            } else {
                BulkEntry::Folder(entry.name.clone())
            }
        } else {
            let key = if entry.id != [0; 16] && entry.allocated > 0 {
                serial
                    .get()
                    .map(|serial| winpath::fold_id(serial, entry.id))
            } else {
                None
            };
            BulkEntry::File {
                name: entry.name.clone(),
                bytes: entry.allocated,
                key,
            }
        });
    })
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn read_i64(bytes: &[u8], at: usize) -> Option<i64> {
    Some(i64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}
