use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

fn name(value: &std::ffi::OsStr) -> io::Result<CString> {
    CString::new(value.as_bytes()).map_err(|_| io::Error::other("invalid file name"))
}

fn open_at(dir: &File, leaf: &CString, flags: i32) -> io::Result<File> {
    let fd = unsafe { libc::openat(dir.as_raw_fd(), leaf.as_ptr(), flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK, 0o600) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    Ok(unsafe { File::from_raw_fd(fd) })
}

pub fn parent(root: &Path, path: &Path) -> io::Result<(File, CString)> {
    let relative = path.strip_prefix(root).map_err(|_| io::Error::other("file outside repository"))?;
    let mut parts = relative.components().peekable();
    let mut dir = File::open(root)?;
    while let Some(part) = parts.next() {
        let Component::Normal(part) = part else { return Err(io::Error::other("invalid relative path")); };
        let part = name(part)?;
        if parts.peek().is_none() { return Ok((dir, part)); }
        dir = open_at(&dir, &part, libc::O_RDONLY | libc::O_DIRECTORY)?;
    }
    Err(io::Error::other("missing file name"))
}

fn regular(file: &File) -> io::Result<()> {
    let meta = file.metadata()?;
    if !meta.is_file() || meta.nlink() != 1 { return Err(io::Error::other("expected a regular file with one link")); }
    Ok(())
}

pub fn replace(root: &Path, path: &Path, original: Option<&str>, proposed: &str) -> io::Result<()> {
    let (dir, leaf) = parent(root, path)?;
    if let Some(original) = original {
        let mut file = open_at(&dir, &leaf, libc::O_RDONLY)?;
        regular(&file)?;
        let mut current = String::new();
        (&mut file).take(original.len() as u64 + 1).read_to_string(&mut current)?;
        if current != original { return Err(io::Error::other("plan changed since the edit was proposed")); }
    }
    let temporary = CString::new(format!(".loopfleet-{}", uuid::Uuid::new_v4())).unwrap();
    let mut file = open_at(&dir, &temporary, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
    let result = (|| {
        file.write_all(proposed.as_bytes())?;
        file.sync_all()?;
        let rc = if original.is_some() {
            unsafe { libc::renameat(dir.as_raw_fd(), temporary.as_ptr(), dir.as_raw_fd(), leaf.as_ptr()) }
        } else {
            unsafe { libc::linkat(dir.as_raw_fd(), temporary.as_ptr(), dir.as_raw_fd(), leaf.as_ptr(), 0) }
        };
        if rc < 0 { return Err(io::Error::last_os_error()); }
        Ok(())
    })();
    unsafe { libc::unlinkat(dir.as_raw_fd(), temporary.as_ptr(), 0); }
    result
}

