//! A running scan's signs of life, and the worker's word that it gave up on the scan. A scan stuck
//! in a file-system call — on a share that stopped answering — can't be stopped, only left behind;
//! this is how it learns, if it ever wakes up, that it must write nothing more.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
};

use crate::{
    data_models::scan_model::{ScanFailure, ScanProblem},
    error_types::ScanError,
    scanner::lock,
};

/// Shared by a running scan, which beats, and the worker, which watches the beats.
#[derive(Debug, Default)]
pub struct Heartbeat {
    beats: AtomicU64,
    given_up: AtomicBool,
    spot: Mutex<Option<Spot>>,
}

/// Where a scan is: the directory it reads, and the folder or book inside it.
#[derive(Debug, Clone, PartialEq)]
pub struct Spot {
    pub directory_id: i64,
    pub root: PathBuf,
    pub path: PathBuf,
}

impl Heartbeat {
    /// A sign that the scan still moves.
    pub fn beat(&self) {
        self.beats.fetch_add(1, Ordering::SeqCst);
    }

    /// The scan enters the directory `directory_id` at `root`.
    pub fn in_directory(&self, directory_id: i64, root: &Path) {
        *lock(&self.spot) = Some(Spot {
            directory_id,
            root: root.to_path_buf(),
            path: root.to_path_buf(),
        });
        self.beat();
    }

    /// The scan reads `path`, in the directory it entered last.
    pub fn at(&self, path: &Path) {
        if let Some(spot) = lock(&self.spot).as_mut() {
            spot.path = path.to_path_buf();
        }
        self.beat();
    }

    pub fn beats(&self) -> u64 {
        self.beats.load(Ordering::SeqCst)
    }

    pub fn spot(&self) -> Option<Spot> {
        lock(&self.spot).clone()
    }

    /// The worker gave up on the scan: it must write nothing more.
    pub fn give_up(&self) {
        self.given_up.store(true, Ordering::SeqCst);
    }

    pub fn given_up(&self) -> bool {
        self.given_up.load(Ordering::SeqCst)
    }

    /// `Err` once the worker gave up on the scan.
    pub fn check(&self) -> Result<(), ScanError> {
        if self.given_up() {
            Err(ScanError::GivenUp)
        } else {
            Ok(())
        }
    }
}

impl Spot {
    /// The scan's failure `error_key` at this spot.
    pub fn failure(&self, error_key: ScanProblem) -> ScanFailure {
        ScanFailure::at(self.directory_id, &self.root, &self.path, error_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sign_of_life_beats_and_the_spot_is_relative_to_its_directory() {
        let heartbeat = Heartbeat::default();
        assert_eq!(heartbeat.spot(), None);
        heartbeat.at(Path::new("/books/before-a-directory.cbz"));
        assert_eq!(heartbeat.spot(), None, "no directory entered yet");

        heartbeat.in_directory(3, Path::new("/books"));
        let entered = heartbeat.spot().unwrap().failure(ScanProblem::Stalled);
        heartbeat.at(Path::new("/books/Manga/Berserk v01.cbz"));
        heartbeat.beat();

        assert_eq!(heartbeat.beats(), 4);
        assert_eq!(entered.relative_path, "");
        assert_eq!(
            heartbeat.spot().unwrap().failure(ScanProblem::Stalled),
            ScanFailure {
                directory_id: 3,
                relative_path: "Manga/Berserk v01.cbz".to_string(),
                error_key: ScanProblem::Stalled,
            }
        );
    }

    #[test]
    fn a_scan_may_go_on_until_it_is_given_up_on() {
        let heartbeat = Heartbeat::default();
        assert!(heartbeat.check().is_ok() && !heartbeat.given_up());

        heartbeat.give_up();

        assert!(heartbeat.given_up());
        assert!(matches!(heartbeat.check(), Err(ScanError::GivenUp)));
    }
}
