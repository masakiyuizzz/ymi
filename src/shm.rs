//! POSIX shared memory for the kitty graphics protocol.
//!
//! Sending a frame inline means base64-encoding a megabyte of pixels and
//! pushing it through the pty, then making the terminal decode it. Kitty's
//! `t=s` transfer instead hands the terminal the name of a shared memory
//! object, so a frame costs one memcpy here and no decoding there - this is
//! what makes video playback possible at terminal refresh rates.

use std::ffi::CString;
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub struct Shm {
    name: String,
    ptr: *mut u8,
    len: usize,
    fd: i32,
}

impl Shm {
    /// Create an object large enough for `len` bytes and map it read/write.
    pub fn new(tag: &str, len: usize) -> io::Result<Shm> {
        if len == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty shm"));
        }
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("/{}-{}-{}", tag, std::process::id(), n);
        let cname = CString::new(name.clone())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bad shm name"))?;

        let fd = unsafe { libc::shm_open(cname.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        if unsafe { libc::ftruncate(fd, len as libc::off_t) } != 0 {
            let err = io::Error::last_os_error();
            unsafe {
                libc::close(fd);
                if let Ok(c) = CString::new(name.clone()) {
                    libc::shm_unlink(c.as_ptr());
                }
            }
            return Err(err);
        }

        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            let err = io::Error::last_os_error();
            unsafe {
                libc::close(fd);
                if let Ok(c) = CString::new(name.clone()) {
                    libc::shm_unlink(c.as_ptr());
                }
            }
            return Err(err);
        }

        Ok(Shm {
            name,
            ptr: ptr as *mut u8,
            len,
            fd,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn write(&mut self, data: &[u8]) {
        let n = data.len().min(self.len);
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), self.ptr, n);
        }
    }
}

impl Drop for Shm {
    fn drop(&mut self) {
        unsafe {
            if !self.ptr.is_null() {
                libc::munmap(self.ptr as *mut libc::c_void, self.len);
            }
            libc::close(self.fd);
            if let Ok(c) = CString::new(self.name.clone()) {
                libc::shm_unlink(c.as_ptr());
            }
        }
    }
}
