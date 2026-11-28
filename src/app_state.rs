//! The state every handler can reach. A handler asks for just the part it needs —
//! `State<Arc<Database>>`, `State<Mailer>`, `State<SetupCode>`, `State<TotpCipher>`,
//! `State<Covers>`, `State<ScanJobs>`, `State<FolderWatch>` or `State<MetadataProviders>` — and
//! the `FromRef` impls hand it over.

use std::sync::Arc;

use axum::extract::FromRef;

use crate::{
    db::Database,
    metadata::MetadataProviders,
    routes::{email_helper::Mailer, registration::SetupCode, totp_cipher::TotpCipher},
    scanner::{cover_image_retriever::Covers, folder_watch::FolderWatch, scan_jobs::ScanJobs},
};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub mailer: Mailer,
    pub setup_code: SetupCode,
    pub totp_cipher: TotpCipher,
    pub covers: Covers,
    pub scans: ScanJobs,
    pub watch: FolderWatch,
    pub metadata: MetadataProviders,
}

impl FromRef<AppState> for Arc<Database> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.db)
    }
}

impl FromRef<AppState> for Mailer {
    fn from_ref(state: &AppState) -> Self {
        state.mailer.clone()
    }
}

impl FromRef<AppState> for SetupCode {
    fn from_ref(state: &AppState) -> Self {
        state.setup_code
    }
}

impl FromRef<AppState> for TotpCipher {
    fn from_ref(state: &AppState) -> Self {
        state.totp_cipher.clone()
    }
}

impl FromRef<AppState> for Covers {
    fn from_ref(state: &AppState) -> Self {
        state.covers.clone()
    }
}

impl FromRef<AppState> for ScanJobs {
    fn from_ref(state: &AppState) -> Self {
        state.scans.clone()
    }
}

impl FromRef<AppState> for FolderWatch {
    fn from_ref(state: &AppState) -> Self {
        state.watch.clone()
    }
}

impl FromRef<AppState> for MetadataProviders {
    fn from_ref(state: &AppState) -> Self {
        state.metadata.clone()
    }
}
