//! Zero-copy buffer pool for efficient memory reuse in download operations.
//!
//! Based on the original Sophon DLL's buffer management, this module provides
//! a pool of reusable buffers to minimize allocations during downloads.

use bytes::{Bytes, BytesMut};
use std::sync::Mutex;

/// Default buffer size for pool (2 MiB - large enough for efficient I/O).
pub const DEFAULT_POOL_BUFFER_SIZE: usize = 2 * 1024 * 1024;
/// Maximum number of buffers to keep in the pool.
pub const MAX_POOL_SIZE: usize = 16;

/// A pool of reusable byte buffers.
pub struct BufferPool {
    buffers: Mutex<Vec<BytesMut>>,
    buffer_size: usize,
}

impl BufferPool {
    /// Create a new buffer pool with the given buffer size.
    pub fn new(buffer_size: usize) -> Self {
        Self {
            buffers: Mutex::new(Vec::with_capacity(MAX_POOL_SIZE)),
            buffer_size,
        }
    }

    /// Acquire a buffer from the pool.
    /// If the pool is empty, a new buffer is allocated.
    pub fn acquire(&self) -> BytesMut {
        let mut buffers = self.buffers.lock().unwrap();
        if let Some(mut buf) = buffers.pop() {
            buf.clear();
            buf
        } else {
            BytesMut::with_capacity(self.buffer_size)
        }
    }

    /// Return a buffer to the pool for reuse.
    /// If the pool is full, the buffer is dropped.
    pub fn release(&self, buf: BytesMut) {
        let mut buffers = self.buffers.lock().unwrap();
        if buffers.len() < MAX_POOL_SIZE {
            buffers.push(buf);
        }
    }

    /// Get the buffer size for this pool.
    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }
}

impl Default for BufferPool {
    fn default() -> Self {
        Self::new(DEFAULT_POOL_BUFFER_SIZE)
    }
}

use std::sync::OnceLock;

/// Global buffer pool for downloads.
static GLOBAL_POOL: OnceLock<BufferPool> = OnceLock::new();

fn get_global_pool() -> &'static BufferPool {
    GLOBAL_POOL.get_or_init(|| BufferPool::new(DEFAULT_POOL_BUFFER_SIZE))
}

/// Acquire a buffer from the global pool.
pub fn acquire_buffer() -> BytesMut {
    get_global_pool().acquire()
}

/// Release a buffer back to the global pool.
pub fn release_buffer(buf: BytesMut) {
    get_global_pool().release(buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_pool_acquire_and_release() {
        let pool = BufferPool::new(1024);
        let mut buf = pool.acquire();
        buf.extend_from_slice(b"hello");
        pool.release(buf);

        let buf2 = pool.acquire();
        assert!(buf2.is_empty());
        assert!(buf2.capacity() >= 1024);
    }

    #[test]
    fn buffer_pool_reuses_buffers() {
        let pool = BufferPool::new(1024);
        let buf = pool.acquire();
        pool.release(buf);

        let buf2 = pool.acquire();
        assert!(buf2.is_empty());
    }

    #[test]
    fn buffer_pool_limits_size() {
        let pool = BufferPool::new(1024);
        // Fill pool beyond capacity
        for _ in 0..MAX_POOL_SIZE + 5 {
            let buf = pool.acquire();
            pool.release(buf);
        }
        // Pool should not grow beyond MAX_POOL_SIZE
        let count = pool.buffers.lock().unwrap().len();
        assert!(count <= MAX_POOL_SIZE);
    }
}
