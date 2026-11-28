use serde::{Deserialize, Serialize};
use std::time::SystemTime;

// === Row id + metadata wrapper ===============================================
// Metadata structs don't store their own row id, but list endpoints need it so
// the client can drill down (e.g. fetch a library's series). This pairs the id
// with the metadata and flattens both into a single JSON object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithId<T: Serialize> {
    pub id: i64,
    #[serde(flatten)]
    pub data: T,
}

// === Database models =========================================================
#[derive(Debug, Clone, Serialize, PartialEq, Deserialize)]
pub enum DatabaseTypes {
    Books,
    Library,
    LibraryElements,
    Series,
    Users,
    Session,
    Totp,
    RecoveryCodes,
    LoginChallenge,
    UserSettings,
    Locales,
    TranslationKeys,
    Translations,
    ReadingModes,
    Directories,
    SeriesMatchKeys,
    ReadingProgress,
    SeriesMetadataSources,
    MetadataCache,
}

impl DatabaseTypes {
    pub fn get_table_name(&self) -> &'static str {
        match self {
            DatabaseTypes::Books => "books",
            DatabaseTypes::Library => "library",
            DatabaseTypes::Series => "series",
            DatabaseTypes::Users => "users",
            DatabaseTypes::LibraryElements => "library_elements",
            DatabaseTypes::Session => "sessions",
            DatabaseTypes::Totp => "totp",
            DatabaseTypes::RecoveryCodes => "recovery_codes",
            DatabaseTypes::LoginChallenge => "login_challenges",
            DatabaseTypes::UserSettings => "user_settings",
            DatabaseTypes::Locales => "locales",
            DatabaseTypes::TranslationKeys => "translation_keys",
            DatabaseTypes::Translations => "translations",
            DatabaseTypes::ReadingModes => "reading_modes",
            DatabaseTypes::Directories => "directories",
            DatabaseTypes::SeriesMatchKeys => "series_match_keys",
            DatabaseTypes::ReadingProgress => "reading_progress",
            DatabaseTypes::SeriesMetadataSources => "series_metadata_sources",
            DatabaseTypes::MetadataCache => "metadata_cache",
        }
    }

    /// The column a row is identified by; `None` for `library_elements`, `translations`,
    /// `reading_modes`, `series_match_keys`, `reading_progress` and `metadata_cache`, whose rows
    /// are identified by two columns.
    pub fn get_key_column(&self) -> Option<&str> {
        match self {
            DatabaseTypes::Books
            | DatabaseTypes::Library
            | DatabaseTypes::Series
            | DatabaseTypes::Users
            | DatabaseTypes::TranslationKeys
            | DatabaseTypes::Directories => Some("id"),
            DatabaseTypes::LibraryElements
            | DatabaseTypes::Translations
            | DatabaseTypes::ReadingModes
            | DatabaseTypes::SeriesMatchKeys
            | DatabaseTypes::ReadingProgress
            | DatabaseTypes::MetadataCache => None,
            DatabaseTypes::Locales => Some("code"),
            DatabaseTypes::Totp | DatabaseTypes::UserSettings => Some("user_id"),
            DatabaseTypes::SeriesMetadataSources => Some("series_id"),
            DatabaseTypes::Session => Some("session_id"),
            DatabaseTypes::RecoveryCodes => Some("recovery_id"),
            DatabaseTypes::LoginChallenge => Some("challenge_id"),
        }
    }
}

// === Library type ============================================================
/// What a library holds; stored as text in `library.library_type`, and sent as `"books"`,
/// `"comics"`, `"manga"`, `"magazines"`, `"documents"` or `"others"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryType {
    Books,
    Comics,
    Manga,
    Magazines,
    Documents,
    Others,
}

impl LibraryType {
    #[allow(
        clippy::should_implement_trait,
        reason = "infallible: unknown values map to `Others`, so `FromStr` doesn't fit"
    )]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "books" => LibraryType::Books,
            "comics" => LibraryType::Comics,
            "manga" => LibraryType::Manga,
            "magazines" => LibraryType::Magazines,
            "documents" => LibraryType::Documents,
            _ => LibraryType::Others,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            LibraryType::Books => "books",
            LibraryType::Comics => "comics",
            LibraryType::Manga => "manga",
            LibraryType::Magazines => "magazines",
            LibraryType::Documents => "documents",
            LibraryType::Others => "others",
        }
    }
}

// === Book format model =======================================================
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BookFormat {
    Pdf,
    Epub,
    Cbz,
    ImageComic,
    None,
}

impl BookFormat {
    #[allow(
        clippy::should_implement_trait,
        reason = "infallible: unknown values map to `None`, so `FromStr` doesn't fit"
    )]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "pdf" => BookFormat::Pdf,
            "epub" => BookFormat::Epub,
            "cbz" => BookFormat::Cbz,
            "image_comic" => BookFormat::ImageComic,
            _ => BookFormat::None,
        }
    }
    pub fn as_str(&self) -> &str {
        match self {
            BookFormat::Pdf => "pdf",
            BookFormat::Epub => "epub",
            BookFormat::Cbz => "cbz",
            BookFormat::ImageComic => "image_comic",
            BookFormat::None => "none",
        }
    }
}

// === Book language model =====================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BookLanguage {
    English,
    German,
    French,
    Spanish,
    Portuguese,
    Italian,
    Russian,
    Chinese,
    Japanese,
    // Catch-all for any other language
    Other(String),
}

impl BookLanguage {
    pub fn from_code(code: &str) -> Self {
        match code.to_lowercase().as_str() {
            "en" => BookLanguage::English,
            "de" => BookLanguage::German,
            "fr" => BookLanguage::French,
            "es" => BookLanguage::Spanish,
            "pt" => BookLanguage::Portuguese,
            "it" => BookLanguage::Italian,
            "ru" => BookLanguage::Russian,
            "zh" => BookLanguage::Chinese,
            "ja" => BookLanguage::Japanese,
            other => BookLanguage::Other(other.to_string()),
        }
    }

    pub fn to_code(&self) -> String {
        match self {
            BookLanguage::English => "en".to_string(),
            BookLanguage::German => "de".to_string(),
            BookLanguage::French => "fr".to_string(),
            BookLanguage::Spanish => "es".to_string(),
            BookLanguage::Portuguese => "pt".to_string(),
            BookLanguage::Italian => "it".to_string(),
            BookLanguage::Russian => "ru".to_string(),
            BookLanguage::Chinese => "zh".to_string(),
            BookLanguage::Japanese => "ja".to_string(),
            BookLanguage::Other(lang) => lang.clone(),
        }
    }

    /// Whether this is the scanner's placeholder for a file that names no language.
    pub fn is_unknown(&self) -> bool {
        matches!(self, BookLanguage::Other(code) if code.eq_ignore_ascii_case("unknown"))
    }
}

// === Reading direction and mode ==============================================

/// The direction a book's own file says it is read in; stored as text in
/// `books.reading_direction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingDirection {
    Rtl,
    Ltr,
}

impl ReadingDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReadingDirection::Rtl => "rtl",
            ReadingDirection::Ltr => "ltr",
        }
    }
}

/// How a user reads a series' books: page by page right to left or left to right, or the pages
/// in one scrolling column; stored as text in `reading_modes.mode`, and sent as `"rtl"` / `"ltr"` /
/// `"continuous"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingMode {
    Rtl,
    Ltr,
    Continuous,
}

impl ReadingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReadingMode::Rtl => "rtl",
            ReadingMode::Ltr => "ltr",
            ReadingMode::Continuous => "continuous",
        }
    }

    /// The mode `name` names, as [`as_str`](Self::as_str) writes it; `None` for any other text.
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Rtl, Self::Ltr, Self::Continuous]
            .into_iter()
            .find(|mode| mode.as_str() == name)
    }
}

impl From<ReadingDirection> for ReadingMode {
    fn from(direction: ReadingDirection) -> Self {
        match direction {
            ReadingDirection::Rtl => ReadingMode::Rtl,
            ReadingDirection::Ltr => ReadingMode::Ltr,
        }
    }
}

/// A user's progress in a book: their place, as its reader counts it — a page, or a flowing EPUB's
/// block, from 0 — out of `total`, and whether they finished it. Without a place for a book they
/// never opened, or only marked by hand.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ReadingProgress {
    pub position: Option<i64>,
    pub total: Option<i64>,
    pub finished: bool,
}

/// A place in a book: `position`, from 0, of the `total` its reader counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub position: i64,
    pub total: i64,
}

// === Book metadata model =====================================================
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookMetadata {
    pub title: String,
    pub author: String,
    pub cover_image: Option<String>,
    pub format: BookFormat,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub file_path: String,
    pub language: BookLanguage,
    pub page_count: Option<i32>,
    pub series: Option<i64>,
    pub volume_number: Option<i64>,
    pub chapter_number: Option<f64>,
    pub page_number: Option<i64>,
    pub file_hash: Option<String>,
    pub last_modified: Option<SystemTime>,
    pub file_size: Option<u64>,
    /// The direction the file states; `None` when it states none.
    pub reading_direction: Option<ReadingDirection>,
    /// The scanned folder the file lies in; `None` for a book scanned before folders were stored,
    /// until adding its folder adopts it.
    pub directory_id: Option<i64>,
    /// What the file looked like when it was last read, to tell a changed one apart without
    /// reading it; never sent to clients.
    pub fingerprint: Option<String>,
}

pub enum BookDatabaseColumns {
    Title,
    Author,
    CoverImage,
    Format,
    Tags,
    Description,
    FilePath,
    Language,
    PageCount,
    Series,
    VolumeNumber,
    ChapterNumber,
    PageNumber,
    FileHash,
    LastModified,
    FileSize,
    ReadingDirection,
    DirectoryId,
    Fingerprint,
}

impl AsRef<str> for BookDatabaseColumns {
    fn as_ref(&self) -> &str {
        match self {
            BookDatabaseColumns::Title => "title",
            BookDatabaseColumns::Author => "author",
            BookDatabaseColumns::CoverImage => "cover_image",
            BookDatabaseColumns::Format => "format",
            BookDatabaseColumns::Tags => "tags",
            BookDatabaseColumns::Description => "description",
            BookDatabaseColumns::FilePath => "file_path",
            BookDatabaseColumns::Language => "language",
            BookDatabaseColumns::PageCount => "page_count",
            BookDatabaseColumns::Series => "series",
            BookDatabaseColumns::VolumeNumber => "volume_number",
            BookDatabaseColumns::ChapterNumber => "chapter_number",
            BookDatabaseColumns::PageNumber => "page_number",
            BookDatabaseColumns::FileHash => "file_hash",
            BookDatabaseColumns::LastModified => "last_modified",
            BookDatabaseColumns::FileSize => "file_size",
            BookDatabaseColumns::ReadingDirection => "reading_direction",
            BookDatabaseColumns::DirectoryId => "directory_id",
            BookDatabaseColumns::Fingerprint => "fingerprint",
        }
    }
}

// === Book series metadata model ==============================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookSeriesMetadata {
    pub name: String,
    /// What the scanner finds the series by: the name it was first found under, lowercased. A
    /// rename leaves it as it is.
    pub match_key: String,
    pub description: Option<String>,
    pub cover_image: Option<String>,
    pub start_release_year: Option<i64>,
    pub end_release_year: Option<i64>,
    /// Never looked up online, nor is any series of a sensitive library.
    pub is_sensitive: bool,
}

pub enum SeriesDatabaseColumns {
    Name,
    Description,
    CoverImage,
    StartReleaseYear,
    EndReleaseYear,
}

impl AsRef<str> for SeriesDatabaseColumns {
    fn as_ref(&self) -> &str {
        match self {
            SeriesDatabaseColumns::Name => "name",
            SeriesDatabaseColumns::Description => "description",
            SeriesDatabaseColumns::CoverImage => "cover_image",
            SeriesDatabaseColumns::StartReleaseYear => "start_release_year",
            SeriesDatabaseColumns::EndReleaseYear => "end_release_year",
        }
    }
}

// === User metadata model =====================================================
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMetadata {
    pub user_id: i64,
    pub username: String,
    pub password_hash: String,
    pub email: String,
    pub email_verified: bool,
    pub role: Role,
    pub created_at: SystemTime,
    pub last_login: Option<SystemTime>,
}

/// What a user may do; stored as text in `users.role`, and sent as `"admin"` / `"member"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    Member,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }
}

pub enum UserDatabaseColumns {
    UserId,
    Username,
    PasswordHash,
    Email,
    EmailVerified,
    Role,
    CreatedAt,
    LastLogin,
}
impl AsRef<str> for UserDatabaseColumns {
    fn as_ref(&self) -> &str {
        match self {
            UserDatabaseColumns::UserId => "id",
            UserDatabaseColumns::Username => "username",
            UserDatabaseColumns::PasswordHash => "password_hash",
            UserDatabaseColumns::Email => "email",
            UserDatabaseColumns::EmailVerified => "email_verified",
            UserDatabaseColumns::Role => "role",
            UserDatabaseColumns::CreatedAt => "created_at",
            UserDatabaseColumns::LastLogin => "last_login",
        }
    }
}

// === Library metadata model ==================================================
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryMetadata {
    pub name: String,
    pub library_type: LibraryType,
    pub cover_image: Option<String>,
    pub description: Option<String>,
    /// None of its series is looked up online.
    pub is_sensitive: bool,
    /// Online lookups of its series include entries a provider marks adult.
    pub adult_metadata: bool,
}

pub enum LibraryDatabaseColumns {
    Name,
    LibraryType,
    CoverImage,
    Description,
    AdultMetadata,
}

impl AsRef<str> for LibraryDatabaseColumns {
    fn as_ref(&self) -> &str {
        match self {
            LibraryDatabaseColumns::Name => "library_name",
            LibraryDatabaseColumns::LibraryType => "library_type",
            LibraryDatabaseColumns::CoverImage => "cover_image",
            LibraryDatabaseColumns::Description => "description",
            LibraryDatabaseColumns::AdultMetadata => "adult_metadata",
        }
    }
}

// === Scanned directory model =================================================
/// A folder the scanner reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Directory {
    /// Canonical: absolute, with every symlink resolved.
    pub path: String,
    /// The library its series join; `None` when it was added without one, or that library was
    /// deleted.
    pub library_id: Option<i64>,
    pub created_at: SystemTime,
}

// === Series-Library connection model =========================================
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesLibraryConnection {
    pub series_id: i64,
    pub library_id: i64,
}

pub enum SeriesLibraryConnectionDatabaseColumns {
    SeriesId,
    LibraryId,
}

impl AsRef<str> for SeriesLibraryConnectionDatabaseColumns {
    fn as_ref(&self) -> &str {
        match self {
            SeriesLibraryConnectionDatabaseColumns::SeriesId => "series_id",
            SeriesLibraryConnectionDatabaseColumns::LibraryId => "library_id",
        }
    }
}

// === Series match key model ==================================================
/// A key the series answers to besides its own `match_key`: one of a series merged into it.
#[derive(Debug, Clone)]
pub struct SeriesMatchKey {
    pub series_id: i64,
    pub match_key: String,
}

// === Parsed Name model =======================================================
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedName {
    pub title: String,
    pub volume_number: Option<i64>,
    pub chapter_number: Option<f64>,
    pub page_number: Option<i64>,
}

// === Sessions ================================================================
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Sessions {
    pub session_id: i64,
    pub user_id: i64,
    pub device_id: String,
    pub device_name: String,
    pub platform: String,
    pub refresh_token: String, // hashed; TODO: rename to `refresh_token_hashed`
    pub session_token: String, // hashed; TODO: rename to `session_token_hashed`
    pub refresh_token_expiration_date: SystemTime,
    pub session_token_expiration_date: SystemTime,
    pub created_at: SystemTime,
    pub last_used_at: SystemTime,
}

/// A login whose password was right but whose TOTP code is still pending. No `Debug`:
/// it carries a token hash.
#[derive(Clone)]
pub struct LoginChallenge {
    pub challenge_id: i64,
    pub user_id: i64,
    pub token_hashed: String,
    pub expires_at: SystemTime,
    pub device_id: String,
    pub device_name: String,
    pub platform: String,
}

pub enum SessionsDatabaseColumns {
    SessionId,
    UserId,
    DeviceId,
    DeviceName,
    Platform,
    RefreshToken,
    SessionToken,
    RefreshTokenExpirationDate,
    SessionTokenExpirationDate,
    CreatedAt,
    LastUsedAt,
}

impl AsRef<str> for SessionsDatabaseColumns {
    fn as_ref(&self) -> &str {
        match self {
            SessionsDatabaseColumns::SessionId => "session_id",
            SessionsDatabaseColumns::UserId => "user_id",
            SessionsDatabaseColumns::DeviceId => "device_id",
            SessionsDatabaseColumns::DeviceName => "device_name",
            SessionsDatabaseColumns::Platform => "platform",
            SessionsDatabaseColumns::RefreshToken => "refresh_token_hashed",
            SessionsDatabaseColumns::SessionToken => "session_token_hashed",
            SessionsDatabaseColumns::RefreshTokenExpirationDate => "refresh_token_valid_until",
            SessionsDatabaseColumns::SessionTokenExpirationDate => "session_token_valid_until",
            SessionsDatabaseColumns::CreatedAt => "created_at",
            SessionsDatabaseColumns::LastUsedAt => "last_used_at",
        }
    }
}

// === Recovery codes ==========================================================
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryCodes {
    pub recovery_id: i64,
    pub user_id: i64,
    pub code_hashed: String,
    pub used: bool,
}

pub enum RecoveryCodesColumns {
    RecoveryId,
    UserId,
    CodeHashed,
    Used,
}

impl AsRef<str> for RecoveryCodesColumns {
    fn as_ref(&self) -> &str {
        match self {
            RecoveryCodesColumns::RecoveryId => "recovery_id",
            RecoveryCodesColumns::UserId => "user_id",
            RecoveryCodesColumns::CodeHashed => "code_hash",
            RecoveryCodesColumns::Used => "used",
        }
    }
}

// === TOTP ====================================================================
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TOTP {
    pub user_id: i64,
    /// Encrypted by `TotpCipher`: base64 of nonce, ciphertext and tag.
    pub authentication_secret: String,
    /// `false` while the setup waits for its first code.
    pub confirmed: bool,
    /// The time step of the last accepted code; only a newer step is accepted.
    pub last_used_step: Option<i64>,
    /// Wrong-code lock: tries in the current round, counted before each check.
    pub failed_attempts: i64,
    /// Wrong-code lock: locks since the last correct code.
    pub lockouts: i64,
    pub last_failed_at: Option<SystemTime>,
    pub created_at: SystemTime,
}

pub enum TOTPDatabaseColumns {
    UserId,
    AuthenticationSecret,
    Confirmed,
    LastUsedStep,
    FailedAttempts,
    Lockouts,
    LastFailedAt,
    CreatedAt,
}

impl AsRef<str> for TOTPDatabaseColumns {
    fn as_ref(&self) -> &str {
        match self {
            TOTPDatabaseColumns::UserId => "user_id",
            TOTPDatabaseColumns::AuthenticationSecret => "authentication_secret",
            TOTPDatabaseColumns::Confirmed => "confirmed",
            TOTPDatabaseColumns::LastUsedStep => "last_used_step",
            TOTPDatabaseColumns::FailedAttempts => "failed_attempts",
            TOTPDatabaseColumns::Lockouts => "lockouts",
            TOTPDatabaseColumns::LastFailedAt => "last_failed_at",
            TOTPDatabaseColumns::CreatedAt => "created_at",
        }
    }
}

// === User settings ===========================================================
/// One row of `user_settings`. `None` is a setting the user hasn't chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSettings {
    pub user_id: i64,
    /// The code of one of the languages in `locales`, such as `de`.
    pub locale: Option<String>,
}

pub enum UserSettingsDatabaseColumns {
    UserId,
    Locale,
}

impl AsRef<str> for UserSettingsDatabaseColumns {
    fn as_ref(&self) -> &str {
        match self {
            UserSettingsDatabaseColumns::UserId => "user_id",
            UserSettingsDatabaseColumns::Locale => "locale",
        }
    }
}

// === Translations ============================================================
/// Who renders a translation: the apps (`frontend/` files) or the server itself (`backend/`
/// files, such as the mail texts); stored as text in `translation_keys.audience`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    Frontend,
    Backend,
}

impl Audience {
    pub fn as_str(&self) -> &'static str {
        match self {
            Audience::Frontend => "frontend",
            Audience::Backend => "backend",
        }
    }
}

// === Email configuration =====================================================
/// How the SMTP connection is encrypted (`SMTP_TLS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpTls {
    StartTls,
    Implicit,
    Off,
}

/// No `Debug`: it carries the SMTP password.
#[derive(Clone)]
pub struct EmailConfig {
    pub host: String,
    pub port: u16,
    pub tls: SmtpTls,
    /// Username and password; `None` for a server that takes mail without a login.
    pub credentials: Option<(String, String)>,
    pub from: String,
    pub from_name: String,
}

// === Query building ==========================================================

/// How multiple column conditions are joined in a `WHERE` clause.
pub enum QuerySeparator {
    And,
    Or,
}

impl QuerySeparator {
    pub fn as_str(&self) -> &'static str {
        match self {
            QuerySeparator::And => " AND ",
            QuerySeparator::Or => " OR ",
        }
    }
}

/// How a column is compared with its bound value in a `WHERE` condition.
pub enum Comparison {
    Equal,
    AtMost,
}

impl Comparison {
    pub fn as_str(&self) -> &'static str {
        match self {
            Comparison::Equal => "=",
            Comparison::AtMost => "<=",
        }
    }
}

// === Emailed codes ===========================================================
/// What an emailed code is for; stored as text in `email_codes.purpose`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodePurpose {
    ResetPassword,
    VerifyEmail,
    ChangeEmail,
}

impl CodePurpose {
    pub fn as_str(&self) -> &'static str {
        match self {
            CodePurpose::ResetPassword => "reset_password",
            CodePurpose::VerifyEmail => "verify_email",
            CodePurpose::ChangeEmail => "change_email",
        }
    }
}

/// One row of `email_codes`. No `Debug`: it carries a code hash.
#[derive(Clone)]
pub struct EmailCode {
    pub user_id: i64,
    pub purpose: CodePurpose,
    /// Argon2 hash of the code; the code itself only ever exists in the email.
    pub code_hash: String,
    /// The address an email change waits to confirm; `None` for the other purposes.
    pub new_email: Option<String>,
    pub expires_at: SystemTime,
    /// Tries on the current code, counted before each check.
    pub attempts: i64,
    pub last_sent_at: SystemTime,
    /// Mails sent since `window_started_at`, for the daily cap.
    pub sent_in_window: i64,
    pub window_started_at: SystemTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_type_travels_in_json_under_the_name_the_database_stores() {
        for library_type in [
            LibraryType::Books,
            LibraryType::Comics,
            LibraryType::Manga,
            LibraryType::Magazines,
            LibraryType::Documents,
            LibraryType::Others,
        ] {
            let sent = serde_json::to_value(library_type).unwrap();
            assert_eq!(sent, serde_json::json!(library_type.as_str()));
            let read: LibraryType = serde_json::from_value(sent).unwrap();
            assert_eq!(read, library_type);
            assert_eq!(LibraryType::from_str(library_type.as_str()), library_type);
        }
    }

    #[test]
    fn a_library_type_by_its_rust_name_or_an_unknown_one_is_refused_in_json() {
        for name in ["Books", "novels", ""] {
            assert!(
                serde_json::from_value::<LibraryType>(serde_json::json!(name)).is_err(),
                "{name:?}"
            );
        }
    }
}
