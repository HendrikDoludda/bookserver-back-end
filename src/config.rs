use std::{
    env, fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error_types::ApplicationSetUpErrors;
use crate::models::{EmailConfig, SmtpTls};
use base64::{engine::general_purpose::STANDARD, Engine};
use directories::ProjectDirs;
use rand::rngs::SysRng;
use rand::TryRng;

/// Server secrets persisted in `secrets.toml` in the data folder.
#[derive(Serialize, Deserialize)]
pub struct Secrets {
    token_signing_key: String,
    previous_token_signing_key: Option<String>,
    previous_token_signing_key_expiration_date: Option<DateTime<Utc>>,
}

impl Secrets {
    /// The signing key as bytes; anything but 32 base64-encoded bytes is refused.
    pub fn signing_key(&self) -> Result<[u8; 32], ApplicationSetUpErrors> {
        STANDARD
            .decode(&self.token_signing_key)
            .ok()
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .ok_or(ApplicationSetUpErrors::InvalidSigningKey)
    }
}

/// The app's identifier, which names the default data folder.
// Changing it moves that folder: an existing install would start empty, with a new signing key
// that can't read its stored TOTP secrets.
fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("com", "hendrikdoludda", "bookserver")
}

/// The folder everything the server writes lies in — the database, the covers and
/// `secrets.toml`: `DATA_DIR`, else the OS's local data folder for the app. It is made if
/// missing, and answered as an absolute path.
pub fn data_folder() -> Result<PathBuf, ApplicationSetUpErrors> {
    data_folder_from(
        |name| env::var(name).ok(),
        || project_dirs().map(|dirs| dirs.data_local_dir().to_path_buf()),
    )
}

/// Reads through `lookup` for the same reason `email_config_from` does: tests pass their own,
/// and their own default folder.
fn data_folder_from(
    lookup: impl Fn(&'static str) -> Option<String>,
    default: impl FnOnce() -> Option<PathBuf>,
) -> Result<PathBuf, ApplicationSetUpErrors> {
    let (folder, failure) = match lookup("DATA_DIR").filter(|value| !value.trim().is_empty()) {
        Some(value) => (
            PathBuf::from(value),
            ApplicationSetUpErrors::InvalidSetting("DATA_DIR"),
        ),
        None => (
            default().ok_or(ApplicationSetUpErrors::NoDataFolder)?,
            ApplicationSetUpErrors::DirectoryCreationFailure,
        ),
    };
    fs::create_dir_all(&folder).map_err(|_| failure)?;
    // Not `canonicalize`: on Windows it answers `\\?\C:\…` paths, and the folders inside are
    // canonicalised where they are compared (`Database::open`, `Covers::in_folder`).
    std::path::absolute(&folder).map_err(|_| ApplicationSetUpErrors::DirectoryCreationFailure)
}

/// Loads `secrets.toml` from `folder`, generating it with a fresh 256-bit key on first run.
/// The file is written to a temp path and then renamed, so a crash mid-write can't leave a
/// half-written secrets file behind.
pub fn load_or_create_secrets(folder: &Path) -> Result<Arc<Secrets>, ApplicationSetUpErrors> {
    fs::create_dir_all(folder).map_err(|_| ApplicationSetUpErrors::DirectoryCreationFailure)?;

    let secrets_path = folder.join("secrets.toml");
    let secrets_tmp_path = folder.join("secrets.toml.tmp");

    if !secrets_path.exists() {
        let contents = create_new_set_up_secrets()?;
        let contents_string = toml::to_string_pretty(&contents)
            .map_err(|_| ApplicationSetUpErrors::SecretConversionFailure)?;
        write_owner_only(&secrets_tmp_path, &contents_string)?;
        fs::rename(&secrets_tmp_path, &secrets_path)
            .map_err(|_| ApplicationSetUpErrors::RenamingFileFailure)?;
    }
    restrict_to_owner(&secrets_path)?;
    let read_content = fs::read_to_string(&secrets_path)
        .map_err(|_| ApplicationSetUpErrors::DocumentReadingFailure)?;
    let secret: Secrets = toml::from_str(&read_content)
        .map_err(|_| ApplicationSetUpErrors::SecretDeserializingError)?;
    Ok(Arc::new(secret))
}

/// Writes a new file that only its owner can read from the moment it exists — a `chmod` after
/// writing would leave it readable in between.
#[cfg(unix)]
fn write_owner_only(path: &Path, contents: &str) -> Result<(), ApplicationSetUpErrors> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};

    // A temp file left by a crash would make `create_new` fail at every start.
    let _ = fs::remove_file(path);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| ApplicationSetUpErrors::DocumentWritingFailure)?;
    file.write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| ApplicationSetUpErrors::DocumentWritingFailure)
}

// On Windows the default data folder lies in the user's own profile, which other users can't read.
#[cfg(not(unix))]
fn write_owner_only(path: &Path, contents: &str) -> Result<(), ApplicationSetUpErrors> {
    fs::write(path, contents).map_err(|_| ApplicationSetUpErrors::DocumentWritingFailure)
}

/// Narrows a `secrets.toml` that other users can read, such as one written before files were
/// created owner-only.
#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> Result<(), ApplicationSetUpErrors> {
    use std::os::unix::fs::PermissionsExt;

    let mode = fs::metadata(path)
        .map_err(|_| ApplicationSetUpErrors::DocumentReadingFailure)?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        tracing::warn!("secrets.toml was readable by other users; restricting it to its owner");
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| ApplicationSetUpErrors::SecretsPermissionFailure)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> Result<(), ApplicationSetUpErrors> {
    Ok(())
}

fn create_new_set_up_secrets() -> Result<Secrets, ApplicationSetUpErrors> {
    let mut key_bytes = [0u8; 32];
    SysRng
        .try_fill_bytes(&mut key_bytes)
        .map_err(|_| ApplicationSetUpErrors::SysRngFailure)?;
    let signing_key = STANDARD.encode(key_bytes);
    let secret = Secrets {
        token_signing_key: signing_key,
        previous_token_signing_key: None,
        previous_token_signing_key_expiration_date: None,
    };
    Ok(secret)
}

/// Where the server listens: `BIND_ADDRESS` (default `127.0.0.1`, this computer only) and `PORT`
/// (default 8080). A value that isn't an IP address or a port is an error naming its setting.
pub fn get_listen_address() -> Result<SocketAddr, ApplicationSetUpErrors> {
    listen_address_from(|name| env::var(name).ok())
}

/// Reads through `lookup` for the same reason `email_config_from` does: tests pass their own.
fn listen_address_from(
    lookup: impl Fn(&'static str) -> Option<String>,
) -> Result<SocketAddr, ApplicationSetUpErrors> {
    let setting = |name: &'static str| lookup(name).filter(|value| !value.trim().is_empty());

    let address = match setting("BIND_ADDRESS") {
        // The server speaks plain HTTP: other devices reach it through `tailscale serve`, which
        // forwards to this address, so by default nothing on the network gets a plain connection.
        None => IpAddr::V4(Ipv4Addr::LOCALHOST),
        Some(value) => value
            .parse()
            .map_err(|_| ApplicationSetUpErrors::InvalidSetting("BIND_ADDRESS"))?,
    };
    let port = match setting("PORT") {
        None => 8080,
        // Port 0 asks the system for any free port, which no client could know.
        Some(value) => value
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or(ApplicationSetUpErrors::InvalidSetting("PORT"))?,
    };
    Ok(SocketAddr::new(address, port))
}

pub fn get_api_key() -> Option<String> {
    env::var("API_KEY").ok()
}

/// Reads the `SMTP_*` settings. `Ok(None)` means email is off because `SMTP_HOST` isn't
/// set; a half-filled configuration is an error, so a typo can't silently switch email off.
pub fn get_email_config() -> Result<Option<EmailConfig>, ApplicationSetUpErrors> {
    email_config_from(|name| env::var(name).ok())
}

/// The lookup is a parameter so tests can pass their own settings: changing the process
/// environment would leak between tests, which run on parallel threads.
fn email_config_from(
    lookup: impl Fn(&'static str) -> Option<String>,
) -> Result<Option<EmailConfig>, ApplicationSetUpErrors> {
    let setting = |name: &'static str| lookup(name).filter(|value| !value.trim().is_empty());
    let required =
        |name: &'static str| setting(name).ok_or(ApplicationSetUpErrors::MissingSetting(name));

    let Some(host) = setting("SMTP_HOST") else {
        return Ok(None);
    };
    let port = required("SMTP_PORT")?
        .parse()
        .map_err(|_| ApplicationSetUpErrors::InvalidSetting("SMTP_PORT"))?;
    let tls = match setting("SMTP_TLS").as_deref() {
        None | Some("starttls") => SmtpTls::StartTls,
        Some("implicit") => SmtpTls::Implicit,
        Some("none") => SmtpTls::Off,
        Some(_) => return Err(ApplicationSetUpErrors::InvalidSetting("SMTP_TLS")),
    };
    let credentials = match (setting("SMTP_USERNAME"), setting("SMTP_PASSWORD")) {
        (Some(username), Some(password)) => Some((username, password)),
        (None, None) => None,
        _ => return Err(ApplicationSetUpErrors::IncompleteSmtpLogin),
    };
    if tls == SmtpTls::Off && credentials.is_some() {
        return Err(ApplicationSetUpErrors::PlaintextSmtpLogin);
    }

    Ok(Some(EmailConfig {
        host,
        port,
        tls,
        credentials,
        from: required("SMTP_FROM")?,
        from_name: required("SMTP_FROM_NAME")?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::collections::HashMap;

    fn parse(
        settings: &[(&'static str, &str)],
    ) -> Result<Option<EmailConfig>, ApplicationSetUpErrors> {
        let settings: HashMap<&str, String> = settings
            .iter()
            .map(|(name, value)| (*name, value.to_string()))
            .collect();
        email_config_from(|name| settings.get(name).cloned())
    }

    const MAILPIT: [(&str, &str); 5] = [
        ("SMTP_HOST", "localhost"),
        ("SMTP_PORT", "1025"),
        ("SMTP_TLS", "none"),
        ("SMTP_FROM", "server@example.com"),
        ("SMTP_FROM_NAME", "Book Server"),
    ];

    fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        let mut settings: Vec<_> = MAILPIT
            .iter()
            .filter(|(name, _)| !extra.iter().any(|(other, _)| other == name))
            .copied()
            .collect();
        settings.extend_from_slice(extra);
        settings
    }

    fn without(name: &str) -> Vec<(&'static str, &'static str)> {
        MAILPIT
            .iter()
            .filter(|(n, _)| *n != name)
            .copied()
            .collect()
    }

    #[test]
    fn no_smtp_host_means_email_is_off() {
        assert!(parse(&[]).unwrap().is_none());
        assert!(parse(&[("SMTP_HOST", "  ")]).unwrap().is_none());
    }

    #[test]
    fn a_mailpit_setup_needs_no_login() {
        let config = parse(&MAILPIT).unwrap().expect("email is on");
        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 1025);
        assert_eq!(config.tls, SmtpTls::Off);
        assert!(config.credentials.is_none());
        assert_eq!(config.from, "server@example.com");
        assert_eq!(config.from_name, "Book Server");
    }

    #[test]
    fn each_tls_mode_is_understood_and_starttls_is_the_default() {
        let login = [("SMTP_USERNAME", "user"), ("SMTP_PASSWORD", "secret")];
        for (value, expected) in [
            ("starttls", SmtpTls::StartTls),
            ("implicit", SmtpTls::Implicit),
        ] {
            let mut settings = with(&[("SMTP_TLS", value)]);
            settings.extend_from_slice(&login);
            let config = parse(&settings).unwrap().unwrap();
            assert_eq!(config.tls, expected, "{value}");
            assert_eq!(
                config.credentials,
                Some(("user".to_string(), "secret".to_string()))
            );
        }

        let config = parse(&without("SMTP_TLS")).unwrap().unwrap();
        assert_eq!(config.tls, SmtpTls::StartTls);
    }

    #[test]
    fn an_unknown_tls_mode_is_refused() {
        let result = parse(&with(&[("SMTP_TLS", "ssl")]));
        assert!(matches!(
            result,
            Err(ApplicationSetUpErrors::InvalidSetting("SMTP_TLS"))
        ));
    }

    #[test]
    fn a_password_is_never_sent_unencrypted() {
        let result = parse(&with(&[
            ("SMTP_USERNAME", "user"),
            ("SMTP_PASSWORD", "secret"),
        ]));
        assert!(matches!(
            result,
            Err(ApplicationSetUpErrors::PlaintextSmtpLogin)
        ));
    }

    #[test]
    fn half_a_login_is_refused() {
        let result = parse(&with(&[
            ("SMTP_TLS", "starttls"),
            ("SMTP_USERNAME", "user"),
        ]));
        assert!(matches!(
            result,
            Err(ApplicationSetUpErrors::IncompleteSmtpLogin)
        ));
    }

    #[test]
    fn a_missing_or_broken_setting_is_named() {
        for name in ["SMTP_PORT", "SMTP_FROM", "SMTP_FROM_NAME"] {
            let result = parse(&without(name));
            assert!(
                matches!(result, Err(ApplicationSetUpErrors::MissingSetting(missing)) if missing == name),
                "{name}"
            );
        }

        let result = parse(&with(&[("SMTP_PORT", "smtp")]));
        assert!(matches!(
            result,
            Err(ApplicationSetUpErrors::InvalidSetting("SMTP_PORT"))
        ));
    }

    fn listen(settings: &[(&'static str, &str)]) -> Result<SocketAddr, ApplicationSetUpErrors> {
        let settings: HashMap<&str, String> = settings
            .iter()
            .map(|(name, value)| (*name, value.to_string()))
            .collect();
        listen_address_from(|name| settings.get(name).cloned())
    }

    #[test]
    fn without_settings_the_server_listens_on_this_computer_only_at_8080() {
        for settings in [vec![], vec![("BIND_ADDRESS", " "), ("PORT", "")]] {
            assert_eq!(listen(&settings).unwrap().to_string(), "127.0.0.1:8080");
        }
    }

    #[test]
    fn an_ipv4_or_ipv6_address_and_a_port_are_listened_on() {
        for (address, expected) in [
            ("127.0.0.1", "127.0.0.1:8443"),
            ("0.0.0.0", "0.0.0.0:8443"),
            ("100.101.102.103", "100.101.102.103:8443"),
            ("::1", "[::1]:8443"),
            ("::", "[::]:8443"),
        ] {
            let listened = listen(&[("BIND_ADDRESS", address), ("PORT", "8443")]).unwrap();
            assert_eq!(listened.to_string(), expected, "{address}");
        }
    }

    #[test]
    fn an_address_that_is_not_a_bare_ip_is_refused_by_name() {
        for address in [
            "localhost",
            "0.0.0.0:8080",
            "256.1.1.1",
            "192.168.1.0/24",
            "[::1]",
        ] {
            assert!(
                matches!(
                    listen(&[("BIND_ADDRESS", address)]),
                    Err(ApplicationSetUpErrors::InvalidSetting("BIND_ADDRESS"))
                ),
                "{address}"
            );
        }
    }

    #[test]
    fn a_port_outside_1_to_65535_is_refused_by_name() {
        for port in ["0", "65536", "-1", "http"] {
            assert!(
                matches!(
                    listen(&[("PORT", port)]),
                    Err(ApplicationSetUpErrors::InvalidSetting("PORT"))
                ),
                "{port}"
            );
        }
    }

    fn data(
        settings: &[(&'static str, &str)],
        default: Option<PathBuf>,
    ) -> Result<PathBuf, ApplicationSetUpErrors> {
        let settings: HashMap<&str, String> = settings
            .iter()
            .map(|(name, value)| (*name, value.to_string()))
            .collect();
        data_folder_from(|name| settings.get(name).cloned(), || default)
    }

    #[test]
    fn data_dir_names_the_data_folder_and_it_is_made() {
        let dir = TempDir::new();
        let wanted = dir.path().join("nested").join("data");
        let default = dir.path().join("default");

        let folder = data(
            &[("DATA_DIR", wanted.to_str().unwrap())],
            Some(default.clone()),
        )
        .unwrap();

        assert_eq!(folder, wanted);
        assert!(folder.is_dir());
        assert!(!default.exists(), "the default folder is left alone");
    }

    #[test]
    fn without_data_dir_the_default_folder_is_made_and_used() {
        for settings in [vec![], vec![("DATA_DIR", "  ")]] {
            let dir = TempDir::new();
            let default = dir.path().join("default");

            let folder = data(&settings, Some(default.clone())).unwrap();

            assert_eq!(folder, default);
            assert!(folder.is_dir());
        }
    }

    #[test]
    fn a_data_dir_that_is_a_file_is_refused_by_name() {
        let dir = TempDir::new();
        let file = dir.write("not-a-folder", b"");

        let result = data(&[("DATA_DIR", file.to_str().unwrap())], None);

        assert!(matches!(
            result,
            Err(ApplicationSetUpErrors::InvalidSetting("DATA_DIR"))
        ));
    }

    #[test]
    fn without_data_dir_or_a_default_folder_the_start_stops() {
        assert!(matches!(
            data(&[], None),
            Err(ApplicationSetUpErrors::NoDataFolder)
        ));
    }

    #[test]
    fn the_default_data_folder_is_named_after_the_permanent_identifier() {
        let folder = project_dirs()
            .expect("a home folder")
            .data_local_dir()
            .to_path_buf();
        let expected = if cfg!(target_os = "macos") {
            Path::new("Library/Application Support/com.hendrikdoludda.bookserver")
        } else if cfg!(windows) {
            Path::new(r"hendrikdoludda\bookserver\data")
        } else {
            Path::new("bookserver")
        };
        assert!(folder.ends_with(expected), "{folder:?}");
    }

    fn signing_key_in(dir: &TempDir) -> [u8; 32] {
        load_or_create_secrets(dir.path())
            .unwrap()
            .signing_key()
            .unwrap()
    }

    #[test]
    fn the_first_start_creates_a_key_and_later_starts_read_the_same_one() {
        let dir = TempDir::new();
        let first = signing_key_in(&dir);
        assert_eq!(signing_key_in(&dir), first);
        assert_ne!(first, [0; 32]);
    }

    #[test]
    fn the_signing_key_must_be_32_base64_encoded_bytes() {
        let secrets = |key: String| Secrets {
            token_signing_key: key,
            previous_token_signing_key: None,
            previous_token_signing_key_expiration_date: None,
        };
        assert_eq!(
            secrets(STANDARD.encode([5; 32])).signing_key().unwrap(),
            [5; 32]
        );
        for bad in [
            STANDARD.encode([5; 31]),
            STANDARD.encode([5; 33]),
            "not base64!".to_string(),
        ] {
            assert!(
                matches!(
                    secrets(bad.clone()).signing_key(),
                    Err(ApplicationSetUpErrors::InvalidSigningKey)
                ),
                "{bad:?}"
            );
        }
    }

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn a_new_secrets_file_is_readable_only_by_its_owner() {
        let dir = TempDir::new();
        load_or_create_secrets(dir.path()).unwrap();
        assert_eq!(mode_of(&dir.path().join("secrets.toml")), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn an_older_readable_secrets_file_is_narrowed_and_keeps_its_key() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new();
        let key = signing_key_in(&dir);
        let path = dir.path().join("secrets.toml");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(signing_key_in(&dir), key);
        assert_eq!(mode_of(&path), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn a_temp_file_left_by_a_crash_does_not_block_the_first_start() {
        let dir = TempDir::new();
        dir.write("secrets.toml.tmp", b"half-written");

        signing_key_in(&dir);

        assert_eq!(mode_of(&dir.path().join("secrets.toml")), 0o600);
        assert!(!dir.path().join("secrets.toml.tmp").exists());
    }
}
