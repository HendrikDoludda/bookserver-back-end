pub mod comic_info;
pub mod cover_image_retriever;
pub mod epub_package;
pub mod folder_scanner;
pub mod folder_watch;
pub mod heartbeat;
pub mod metadata_fetcher;
pub mod scan_jobs;

use std::sync::{Mutex, MutexGuard, PoisonError};

/// The lock on `mutex`, also after a thread panicked holding it: every change made under the
/// scanner's locks is one insert, removal, call or assignment, so a panic leaves nothing half done.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
