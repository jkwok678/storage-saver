//! Directory reads through `getattrlistbulk`, so a folder's files need no `lstat` of their own.

use std::cell::RefCell;
use std::ffi::CString;
use std::io::{self, ErrorKind};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::platform::BulkEntry;

// Not in the libc crate. Values from <sys/attr.h> and <sys/vnode.h>.
const ATTR_CMN_ERROR: u32 = 0x2000_0000;
const VREG: u32 = 1;
const VDIR: u32 = 2;

const BULK_COMMON: u32 = libc::ATTR_CMN_RETURNED_ATTRS
    | libc::ATTR_CMN_NAME
    | libc::ATTR_CMN_DEVID
    | libc::ATTR_CMN_OBJTYPE
    | libc::ATTR_CMN_FILEID
    | ATTR_CMN_ERROR;
const BULK_FILE: u32 = libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_ALLOCSIZE;

thread_local! {
    /// Reused across reads. `u64` keeps it 8-byte aligned.
    static BULK_BUFFER: RefCell<Vec<u64>> = RefCell::new(vec![0; 64 * 1024 / 8]);
}

/// List `dir` with `getattrlistbulk`. One call returns the type, allocated size, link count
/// and file id of a whole batch of entries, so files cost no `lstat` of their own.
pub(crate) fn read_dir_bulk(dir: &Path, mut each: impl FnMut(BulkEntry)) -> io::Result<()> {
    let path = CString::new(dir.as_os_str().as_bytes())
        .map_err(|err| io::Error::new(ErrorKind::InvalidInput, err))?;
    // SAFETY: path is a valid C string.
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd was just opened and nothing else owns it.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };

    let mut request = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: BULK_COMMON,
        volattr: 0,
        dirattr: 0,
        fileattr: BULK_FILE,
        forkattr: 0,
    };
    BULK_BUFFER.with_borrow_mut(|buffer| {
        let size = buffer.len() * 8;
        loop {
            // SAFETY: request and buffer outlive the call, and `size` is the buffer's length in bytes.
            let count = unsafe {
                libc::getattrlistbulk(
                    fd.as_raw_fd(),
                    (&raw mut request).cast(),
                    buffer.as_mut_ptr().cast(),
                    size,
                    0,
                )
            };
            if count < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }
            if count == 0 {
                return Ok(());
            }
            // SAFETY: the buffer is `size` initialized bytes, and u8 has no alignment needs.
            let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), size) };
            let mut offset = 0;
            for _ in 0..count {
                let length = read_u32(bytes, offset).unwrap_or(0) as usize;
                let Some(entry) = bytes.get(offset..offset + length).filter(|_| length > 0) else {
                    return Err(io::Error::new(
                        ErrorKind::InvalidData,
                        "getattrlistbulk returned a malformed entry",
                    ));
                };
                if let Some(entry) = parse_bulk_entry(entry) {
                    each(entry);
                }
                offset += length;
            }
        }
    })
}

/// Attributes follow the entry's length and returned-attribute set, in bit order,
/// each 4-byte aligned. Ones the file system did not return are left out.
fn parse_bulk_entry(entry: &[u8]) -> Option<BulkEntry> {
    let mut at = 4;
    let take_u32 = |at: &mut usize| {
        let value = read_u32(entry, *at);
        *at += 4;
        value
    };
    let common = take_u32(&mut at)?;
    let _volume = take_u32(&mut at)?;
    let _dir = take_u32(&mut at)?;
    let file = take_u32(&mut at)?;
    let _fork = take_u32(&mut at)?;

    let mut name: &[u8] = &[];
    if common & libc::ATTR_CMN_NAME != 0 {
        // attrreference_t: offset from the reference itself, then length with the NUL.
        let reference = at;
        let offset = read_u32(entry, at)? as i32;
        let length = read_u32(entry, at + 4)? as usize;
        at += 8;
        let start = reference.checked_add_signed(offset as isize)?;
        name = entry.get(start..start + length)?;
        name = name.strip_suffix(&[0]).unwrap_or(name);
    }
    let dev = if common & libc::ATTR_CMN_DEVID != 0 {
        take_u32(&mut at)?
    } else {
        0
    };
    let kind = if common & libc::ATTR_CMN_OBJTYPE != 0 {
        take_u32(&mut at)?
    } else {
        0
    };
    let file_id = if common & libc::ATTR_CMN_FILEID != 0 {
        let value = read_u64(entry, at)?;
        at += 8;
        value
    } else {
        0
    };
    if common & ATTR_CMN_ERROR != 0 {
        let errno = take_u32(&mut at)?;
        if errno != 0 {
            return Some(BulkEntry::Error(errno as i32));
        }
    }

    Some(match kind {
        VDIR => BulkEntry::Folder(String::from_utf8_lossy(name).into_owned()),
        VREG => {
            let links = if file & libc::ATTR_FILE_LINKCOUNT != 0 {
                take_u32(&mut at)?
            } else {
                1
            };
            let bytes = if file & libc::ATTR_FILE_ALLOCSIZE != 0 {
                read_u64(entry, at)?
            } else {
                0
            };
            BulkEntry::File {
                name: String::from_utf8_lossy(name).into_owned(),
                bytes,
                key: (links > 1).then_some((u64::from(dev), file_id)),
            }
        }
        _ => BulkEntry::Other,
    })
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_ne_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}
