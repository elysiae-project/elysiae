//! Hierarchical concurrency control for Sophon downloads.
//!
//! This module implements a layered semaphore system inspired by the original
//! Sophon DLL's concurrency model (`max_concurrent_tasks`,
//! `chunk_max_concurrent_tasks`, `concurrent_verification_tasks`, etc.).

use std::sync::Arc;
use tokio::sync::{Semaphore, SemaphorePermit};

/// Default maximum concurrent download tasks globally.
pub const DEFAULT_MAX_CONCURRENT_TASKS: usize = 64;
/// Default maximum concurrent chunk downloads.
pub const DEFAULT_CHUNK_MAX_CONCURRENT: usize = 32;
/// Default maximum concurrent patch (ldiff) tasks.
pub const DEFAULT_LDIFF_MAX_CONCURRENT: usize = 8;
/// Default maximum concurrent verification tasks.
pub const DEFAULT_CONCURRENT_VERIFICATION: usize = 16;

/// Hierarchical concurrency manager with layered semaphore control.
pub struct ConcurrencyManager {
    /// Global limit on all concurrent tasks.
    global: Arc<Semaphore>,
    /// Limit on concurrent chunk downloads.
    chunk: Arc<Semaphore>,
    /// Limit on concurrent patch (ldiff) tasks.
    ldiff: Arc<Semaphore>,
    /// Limit on concurrent verification tasks.
    verification: Arc<Semaphore>,
}

impl ConcurrencyManager {
    pub fn new(
        max_concurrent: usize,
        chunk_max_concurrent: usize,
        ldiff_max_concurrent: usize,
        verification_max: usize,
    ) -> Self {
        Self {
            global: Arc::new(Semaphore::new(max_concurrent)),
            chunk: Arc::new(Semaphore::new(chunk_max_concurrent)),
            ldiff: Arc::new(Semaphore::new(ldiff_max_concurrent)),
            verification: Arc::new(Semaphore::new(verification_max)),
        }
    }

    /// Acquire a permit from the global semaphore.
    pub async fn acquire_global(&self) -> Result<SemaphorePermit<'_>, tokio::sync::AcquireError> {
        self.global.acquire().await
    }

    /// Acquire a permit from the chunk semaphore.
    pub async fn acquire_chunk(&self) -> Result<SemaphorePermit<'_>, tokio::sync::AcquireError> {
        self.chunk.acquire().await
    }

    /// Acquire a permit from the ldiff semaphore.
    pub async fn acquire_ldiff(&self) -> Result<SemaphorePermit<'_>, tokio::sync::AcquireError> {
        self.ldiff.acquire().await
    }

    /// Acquire a permit from the verification semaphore.
    pub async fn acquire_verification(
        &self,
    ) -> Result<SemaphorePermit<'_>, tokio::sync::AcquireError> {
        self.verification.acquire().await
    }

    /// Try to acquire a global permit without waiting.
    pub fn try_acquire_global(&self) -> Option<SemaphorePermit<'_>> {
        self.global.try_acquire().ok()
    }

    /// Try to acquire a chunk permit without waiting.
    pub fn try_acquire_chunk(&self) -> Option<SemaphorePermit<'_>> {
        self.chunk.try_acquire().ok()
    }

    /// Try to acquire a verification permit without waiting.
    pub fn try_acquire_verification(&self) -> Option<SemaphorePermit<'_>> {
        self.verification.try_acquire().ok()
    }

    /// Get the number of available global permits.
    pub fn available_global(&self) -> usize {
        self.global.available_permits()
    }

    /// Get the number of available chunk permits.
    pub fn available_chunk(&self) -> usize {
        self.chunk.available_permits()
    }

    /// Get the number of available verification permits.
    pub fn available_verification(&self) -> usize {
        self.verification.available_permits()
    }
}

impl Default for ConcurrencyManager {
    fn default() -> Self {
        Self::new(
            DEFAULT_MAX_CONCURRENT_TASKS,
            DEFAULT_CHUNK_MAX_CONCURRENT,
            DEFAULT_LDIFF_MAX_CONCURRENT,
            DEFAULT_CONCURRENT_VERIFICATION,
        )
    }
}

/// Shared reference type for concurrency manager.
pub type SharedConcurrencyManager = Arc<ConcurrencyManager>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits() {
        let mgr = ConcurrencyManager::default();
        assert_eq!(mgr.available_global(), DEFAULT_MAX_CONCURRENT_TASKS);
        assert_eq!(mgr.available_chunk(), DEFAULT_CHUNK_MAX_CONCURRENT);
    }

    #[tokio::test]
    async fn acquire_and_release_global() {
        let mgr = ConcurrencyManager::default();
        let initial = mgr.available_global();
        let permit = mgr.acquire_global().await.unwrap();
        assert_eq!(mgr.available_global(), initial - 1);
        drop(permit);
        assert_eq!(mgr.available_global(), initial);
    }

    #[tokio::test]
    async fn acquire_and_release_chunk() {
        let mgr = ConcurrencyManager::default();
        let initial = mgr.available_chunk();
        let permit = mgr.acquire_chunk().await.unwrap();
        assert_eq!(mgr.available_chunk(), initial - 1);
        drop(permit);
        assert_eq!(mgr.available_chunk(), initial);
    }

    #[tokio::test]
    async fn try_acquire_succeeds_when_available() {
        let mgr = ConcurrencyManager::default();
        let permit = mgr.try_acquire_global();
        assert!(permit.is_some());
        drop(permit);
    }

    #[tokio::test]
    async fn multiple_concurrent_acquisitions() {
        let mgr = Arc::new(ConcurrencyManager::default());
        let mut handles = Vec::new();

        for _ in 0..5 {
            let m = mgr.clone();
            handles.push(tokio::spawn(async move {
                let _permit = m.acquire_global().await.unwrap();
            }));
        }

        for h in handles {
            h.await.unwrap();
        }
    }
}
