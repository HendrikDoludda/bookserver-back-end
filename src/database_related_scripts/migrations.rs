use crate::database_related_scripts::db::logged;
use crate::error_types::DatabaseError;
use rusqlite::{Connection, OptionalExtension};
use unicode_normalization::UnicodeNormalization;

// A single migration: a version number, a human-readable label, the SQL
// to run, and a Rust step to run after it for a change SQLite's SQL can't
// make. Versions must be unique and applied in ascending order.
struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
    step: Option<Step>,
}

/// Rust a migration runs after its SQL, inside the migration's transaction.
type Step = fn(&Connection) -> Result<(), DatabaseError>;

// `include_str!` reads the file at COMPILE time and bakes its contents into
// the binary as a &'static str. That means the migrations/ folder does not
// need to ship next to the executable — the SQL travels inside it.
// The path is resolved relative to THIS source file.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial_schema",
        sql: include_str!("../../migrations/0001_initial_schema.sql"),
        step: None,
    },
    Migration {
        version: 2,
        name: "reading_modes",
        sql: include_str!("../../migrations/0002_reading_modes.sql"),
        step: None,
    },
    Migration {
        version: 3,
        name: "books_series_foreign_key",
        sql: include_str!("../../migrations/0003_books_series_foreign_key.sql"),
        step: None,
    },
    Migration {
        version: 4,
        name: "cover_file_names",
        sql: include_str!("../../migrations/0004_cover_file_names.sql"),
        step: None,
    },
    Migration {
        version: 5,
        name: "series_match_key",
        sql: include_str!("../../migrations/0005_series_match_key.sql"),
        step: None,
    },
    Migration {
        version: 6,
        name: "directories",
        sql: include_str!("../../migrations/0006_directories.sql"),
        step: None,
    },
    Migration {
        version: 7,
        name: "book fingerprints",
        sql: include_str!("../../migrations/0007_book_fingerprints.sql"),
        step: None,
    },
    Migration {
        version: 8,
        name: "series_match_key_composed",
        sql: include_str!("../../migrations/0008_series_match_key_composed.sql"),
        step: Some(compose_match_keys),
    },
    Migration {
        version: 9,
        name: "series_match_keys",
        sql: include_str!("../../migrations/0009_series_match_keys.sql"),
        step: None,
    },
];

/// Migration 8: every series' match key composed (NFC), as the scanner now builds its keys.
fn compose_match_keys(conn: &Connection) -> Result<(), DatabaseError> {
    let keys = conn
        .prepare("SELECT id, match_key FROM series ORDER BY id")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(logged("reading", "series", DatabaseError::QueryFailure))?;
    for (id, key) in keys {
        let composed: String = key.nfc().collect();
        if composed != key {
            conn.execute(
                "UPDATE series SET match_key = ?1 WHERE id = ?2",
                rusqlite::params![composed, id],
            )
            .map_err(logged(
                "composing the match key of",
                "series",
                DatabaseError::OperationFailure,
            ))?;
        }
    }
    Ok(())
}

// Apply every migration that has not yet been recorded in `schema_migrations`.
// Each migration runs inside its own transaction: if anything inside the SQL
// or its step fails, SQLite rolls the whole thing back so the DB never ends up
// in a half-migrated state.
pub fn run(conn: &mut Connection) -> Result<(), DatabaseError> {
    // 1. Bookkeeping table. Created with the same idempotent style as the
    //    real tables, so this function is safe to call on every startup.
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version    INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
        )",
        [],
    )
    .map_err(logged(
        "creating",
        "schema_migrations",
        DatabaseError::DatabaseInitializationFailure,
    ))?;

    // 2. Walk the migrations in order and apply any that are missing.
    for m in MIGRATIONS {
        let already_applied = conn
            .query_row(
                "SELECT 1 FROM schema_migrations WHERE version = ?1",
                [m.version],
                |_| Ok(()),
            )
            .optional()
            .map_err(logged(
                "reading",
                "schema_migrations",
                DatabaseError::OperationFailure,
            ))?
            .is_some();

        if already_applied {
            continue;
        }

        let tx = conn.transaction().map_err(logged(
            "starting a transaction for migration",
            m.name,
            DatabaseError::TransactionFailure,
        ))?;

        tx.execute_batch(m.sql).map_err(logged(
            "applying migration",
            m.name,
            DatabaseError::DatabaseInitializationFailure,
        ))?;
        if let Some(step) = m.step {
            step(&tx)?;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        tx.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
            rusqlite::params![m.version, now],
        )
        .map_err(logged(
            "recording migration",
            m.name,
            DatabaseError::ConnectionExecutableFailure,
        ))?;

        tx.commit().map_err(logged(
            "committing migration",
            m.name,
            DatabaseError::OperationCommitFailure,
        ))?;

        tracing::info!("Applied migration {:04} {}", m.version, m.name);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database_related_scripts::db::configure;
    use crate::test_support::{insert_user, memory_connection, migrated_connection};
    use rusqlite::params;

    #[test]
    fn deleting_a_user_deletes_only_their_auth_rows() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let bob = insert_user(&conn, "bob");

        for user_id in [alice, bob] {
            let unique = format!("for-{user_id}");
            conn.execute(
                "INSERT INTO email_codes (user_id, purpose, code_hash, expires_at, \
                 last_sent_at, sent_in_window, window_started_at) \
                 VALUES (?1, 'reset_password', ?2, 0, 0, 1, 0)",
                params![user_id, unique],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (user_id, device_id, device_name, platform, \
                 refresh_token_hashed, refresh_token_valid_until, session_token_hashed, \
                 session_token_valid_until, last_used_at) \
                 VALUES (?1, ?2, 'phone', 'ios', ?2, 0, ?2, 0, 0)",
                params![user_id, unique],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO recovery_codes (user_id, code_hash) VALUES (?1, ?2)",
                params![user_id, unique],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO totp (user_id, authentication_secret) VALUES (?1, 'secret')",
                params![user_id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO login_challenges (user_id, token_hashed, expires_at, device_id, \
                 device_name, platform) VALUES (?1, ?2, 0, 'device', 'phone', 'ios')",
                params![user_id, unique],
            )
            .unwrap();
        }

        conn.execute("DELETE FROM users WHERE id = ?1", [alice])
            .unwrap();

        for table in [
            "email_codes",
            "sessions",
            "recovery_codes",
            "totp",
            "login_challenges",
        ] {
            let count = |user_id: i64| -> i64 {
                conn.query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE user_id = ?1"),
                    [user_id],
                    |row| row.get(0),
                )
                .unwrap()
            };
            assert_eq!(
                count(alice),
                0,
                "{table} still has rows of the deleted user"
            );
            assert_eq!(count(bob), 1, "{table} lost rows of another user");
        }
    }

    #[test]
    fn deleting_a_user_deletes_only_their_settings() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let bob = insert_user(&conn, "bob");
        for user_id in [alice, bob] {
            conn.execute(
                "INSERT INTO user_settings (user_id, locale) VALUES (?1, 'de')",
                [user_id],
            )
            .unwrap();
        }

        conn.execute("DELETE FROM users WHERE id = ?1", [alice])
            .unwrap();

        let owners: Vec<i64> = conn
            .prepare("SELECT user_id FROM user_settings")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(owners, [bob]);
    }

    fn add_session(
        conn: &Connection,
        user_id: i64,
        device: &str,
        session: &str,
        refresh: &str,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO sessions (user_id, device_id, device_name, platform, \
             refresh_token_hashed, refresh_token_valid_until, session_token_hashed, \
             session_token_valid_until, last_used_at) \
             VALUES (?1, ?2, 'phone', 'ios', ?3, 0, ?4, 0, 0)",
            params![user_id, device, refresh, session],
        )
    }

    #[test]
    fn a_user_has_at_most_one_session_per_device() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let bob = insert_user(&conn, "bob");
        add_session(&conn, alice, "device-1", "s1", "r1").unwrap();

        assert!(add_session(&conn, alice, "device-1", "s2", "r2").is_err());
        assert!(
            add_session(&conn, bob, "device-1", "s3", "r3").is_ok(),
            "the same device id on another account is a different session"
        );
    }

    #[test]
    fn session_and_refresh_tokens_are_unique() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        add_session(&conn, alice, "device-1", "same-session", "r1").unwrap();
        add_session(&conn, alice, "device-2", "s2", "same-refresh").unwrap();

        assert!(add_session(&conn, alice, "device-3", "same-session", "r3").is_err());
        assert!(add_session(&conn, alice, "device-4", "s4", "same-refresh").is_err());
    }

    /// Whether a column refuses NULL, read from the schema itself.
    fn refuses_null(conn: &Connection, table: &str, column: &str) -> bool {
        conn.query_row(
            "SELECT \"notnull\" FROM pragma_table_info(?1) WHERE name = ?2",
            [table, column],
            |row| row.get(0),
        )
        .unwrap_or_else(|err| panic!("{table}.{column}: {err}"))
    }

    #[test]
    fn a_book_table_column_allows_null_exactly_when_its_rust_field_is_an_option() {
        let conn = memory_connection();
        let required = [
            ("books", "title"),
            ("books", "author"),
            ("books", "format"),
            ("books", "language"),
            ("books", "tags"),
            ("books", "file_path"),
            ("library", "library_name"),
            ("library", "library_type"),
            ("series", "name"),
            ("series", "match_key"),
            ("directories", "path"),
            ("directories", "created_at"),
        ];
        let optional = [
            ("books", "cover_image"),
            ("books", "description"),
            ("books", "page_count"),
            ("books", "volume_number"),
            ("books", "chapter_number"),
            ("books", "page_number"),
            ("books", "file_hash"),
            ("books", "last_modified"),
            ("books", "file_size"),
            ("books", "series"),
            ("books", "reading_direction"),
            ("books", "directory_id"),
            ("books", "fingerprint"),
            ("library", "cover_image"),
            ("library", "description"),
            ("series", "description"),
            ("series", "cover_image"),
            ("series", "start_release_year"),
            ("series", "end_release_year"),
            ("directories", "library_id"),
        ];
        for (table, column) in required {
            assert!(
                refuses_null(&conn, table, column),
                "{table}.{column} must refuse NULL"
            );
        }
        for (table, column) in optional {
            assert!(
                !refuses_null(&conn, table, column),
                "{table}.{column} must allow NULL"
            );
        }
        // A new column has to be sorted into one of the lists above.
        for table in ["books", "library", "series", "directories"] {
            let columns: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name != 'id'",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            let listed = required
                .iter()
                .chain(&optional)
                .filter(|(listed_table, _)| *listed_table == table)
                .count();
            assert_eq!(columns, listed as i64, "every column of {table} is listed");
        }
    }

    #[test]
    fn a_database_of_the_first_schema_gets_the_later_ones_and_keeps_its_books() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        // As a server of the first schema left it.
        conn.execute_batch(MIGRATIONS[0].sql).unwrap();
        conn.execute(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, \
             applied_at INTEGER NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (1, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO books (title, author, format, language, tags, file_path) \
             VALUES ('Berserk', 'Kentaro Miura', 'cbz', 'ja', '', '/books/Berserk v01.cbz')",
            [],
        )
        .unwrap();
        let versions = |conn: &Connection| -> Vec<i64> {
            conn.prepare("SELECT version FROM schema_migrations ORDER BY version")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };

        run(&mut conn).unwrap();

        let (title, direction): (String, Option<String>) = conn
            .query_row("SELECT title, reading_direction FROM books", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(title, "Berserk");
        assert_eq!(direction, None, "a book scanned before states no direction");
        let modes: i64 = conn
            .query_row("SELECT COUNT(*) FROM reading_modes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(modes, 0);
        let every_version: Vec<i64> = MIGRATIONS.iter().map(|m| m.version).collect();
        assert_eq!(versions(&conn), every_version);

        run(&mut conn).unwrap();
        assert_eq!(
            versions(&conn),
            every_version,
            "the next start applies nothing again"
        );
    }

    #[test]
    fn a_books_reading_direction_is_rtl_ltr_or_none() {
        let conn = memory_connection();
        let insert = |direction: Option<&str>, path: &str| {
            conn.execute(
                "INSERT INTO books (title, author, format, language, tags, file_path, \
                 reading_direction) VALUES ('t', 'a', 'cbz', 'ja', '', ?1, ?2)",
                params![path, direction],
            )
        };
        assert!(insert(Some("rtl"), "/books/a.cbz").is_ok());
        assert!(insert(Some("ltr"), "/books/b.cbz").is_ok());
        assert!(insert(None, "/books/c.cbz").is_ok());
        for refused in ["continuous", "RTL", ""] {
            assert!(
                insert(Some(refused), &format!("/books/{refused}-d.cbz")).is_err(),
                "{refused:?} must be refused"
            );
        }
    }

    fn insert_series(conn: &Connection, name: &str) -> i64 {
        conn.execute("INSERT INTO series (name) VALUES (?1)", [name])
            .unwrap();
        conn.last_insert_rowid()
    }

    fn add_reading_mode(
        conn: &Connection,
        user_id: i64,
        series_id: i64,
        mode: &str,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO reading_modes (user_id, series_id, mode) VALUES (?1, ?2, ?3)",
            params![user_id, series_id, mode],
        )
    }

    #[test]
    fn a_reading_mode_is_rtl_ltr_or_continuous_and_one_per_user_and_series() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let berserk = insert_series(&conn, "berserk");

        for refused in ["sideways", "RTL", ""] {
            assert!(
                add_reading_mode(&conn, alice, berserk, refused).is_err(),
                "{refused:?} must be refused"
            );
        }
        assert!(add_reading_mode(&conn, alice, berserk, "continuous").is_ok());
        assert!(
            add_reading_mode(&conn, alice, berserk, "rtl").is_err(),
            "one mode per user and series"
        );
        assert!(
            add_reading_mode(&conn, alice, berserk + 1, "rtl").is_err(),
            "only a series that exists"
        );
    }

    #[test]
    fn deleting_a_user_or_a_series_deletes_only_their_reading_modes() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let bob = insert_user(&conn, "bob");
        let berserk = insert_series(&conn, "berserk");
        let monster = insert_series(&conn, "monster");
        for user_id in [alice, bob] {
            for series_id in [berserk, monster] {
                add_reading_mode(&conn, user_id, series_id, "rtl").unwrap();
            }
        }
        let left = || -> Vec<(i64, i64)> {
            conn.prepare("SELECT user_id, series_id FROM reading_modes ORDER BY user_id, series_id")
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };

        conn.execute("DELETE FROM users WHERE id = ?1", [alice])
            .unwrap();
        assert_eq!(left(), [(bob, berserk), (bob, monster)]);

        conn.execute("DELETE FROM series WHERE id = ?1", [berserk])
            .unwrap();
        assert_eq!(left(), [(bob, monster)]);
    }

    #[test]
    fn a_new_user_is_a_member_and_only_admin_or_member_is_stored() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let role = || -> String {
            conn.query_row("SELECT role FROM users WHERE id = ?1", [alice], |row| {
                row.get(0)
            })
            .unwrap()
        };
        assert_eq!(role(), "member");

        let set = |role: &str| {
            conn.execute(
                "UPDATE users SET role = ?1 WHERE id = ?2",
                params![role, alice],
            )
        };
        assert!(set("admin").is_ok());
        assert_eq!(role(), "admin");
        for refused in ["owner", "Admin", ""] {
            assert!(set(refused).is_err(), "{refused:?} must be refused");
        }
        assert_eq!(role(), "admin", "a refused update changes nothing");
    }

    /// Tests use codes from `qaa`–`qtz`, which ISO 639-2 reserves for local use, so they never
    /// clash with a real language.
    fn add_language(
        conn: &Connection,
        code: &str,
        is_default: bool,
        enabled: bool,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO locales (code, display_name, is_default, enabled, sort_order) \
             VALUES (?1, ?1, ?2, ?3, 1)",
            params![code, is_default, enabled],
        )
    }

    fn add_key(
        conn: &Connection,
        audience: &str,
        namespace: &str,
        name: &str,
    ) -> rusqlite::Result<i64> {
        conn.execute(
            "INSERT INTO translation_keys (audience, namespace, name) VALUES (?1, ?2, ?3)",
            params![audience, namespace, name],
        )?;
        Ok(conn.last_insert_rowid())
    }

    fn add_text(conn: &Connection, code: &str, key_id: i64) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO translations (locale_code, key_id, value) VALUES (?1, ?2, 'text')",
            params![code, key_id],
        )
    }

    fn default_language(conn: &Connection) -> Option<String> {
        conn.query_row("SELECT code FROM locales WHERE is_default", [], |row| {
            row.get(0)
        })
        .optional()
        .unwrap()
    }

    #[test]
    fn at_most_one_language_is_the_default() {
        let conn = migrated_connection();
        add_language(&conn, "qaa", true, true).unwrap();
        add_language(&conn, "qab", false, true).unwrap();

        assert!(
            add_language(&conn, "qac", true, true).is_err(),
            "a second default is refused"
        );
        let second = conn.execute(
            "UPDATE locales SET is_default = TRUE WHERE code = 'qab'",
            [],
        );
        assert!(
            second.is_err(),
            "making a second language the default is refused"
        );
        assert_eq!(default_language(&conn).as_deref(), Some("qaa"));
    }

    #[test]
    fn the_default_moves_once_the_old_one_is_cleared() {
        let conn = migrated_connection();
        add_language(&conn, "qaa", true, true).unwrap();
        add_language(&conn, "qab", false, true).unwrap();

        conn.execute(
            "UPDATE locales SET is_default = FALSE WHERE code = 'qaa'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE locales SET is_default = TRUE WHERE code = 'qab'",
            [],
        )
        .unwrap();

        assert_eq!(default_language(&conn).as_deref(), Some("qab"));
    }

    #[test]
    fn the_default_language_is_enabled() {
        let conn = migrated_connection();
        assert!(
            add_language(&conn, "qaa", true, false).is_err(),
            "a disabled default is refused"
        );

        add_language(&conn, "qaa", true, true).unwrap();
        let disabled = conn.execute("UPDATE locales SET enabled = FALSE WHERE code = 'qaa'", []);
        assert!(disabled.is_err(), "disabling the default is refused");
    }

    #[test]
    fn a_key_is_frontend_or_backend_and_its_name_is_unique_in_its_namespace() {
        let conn = migrated_connection();
        assert!(add_key(&conn, "admin", "test", "title").is_err());

        add_key(&conn, "frontend", "test", "title").unwrap();
        assert!(add_key(&conn, "frontend", "test", "title").is_err());
        assert!(
            add_key(&conn, "backend", "test", "title").is_err(),
            "the other side doesn't make it another key"
        );
        assert!(
            add_key(&conn, "frontend", "other", "title").is_ok(),
            "the same name in another namespace is another key"
        );
    }

    #[test]
    fn a_text_belongs_to_an_existing_language_and_key_and_exists_once() {
        let conn = migrated_connection();
        add_language(&conn, "qaa", false, true).unwrap();
        let key = add_key(&conn, "frontend", "test", "title").unwrap();

        assert!(add_text(&conn, "qzz", key).is_err(), "an unknown language");
        assert!(add_text(&conn, "qaa", key + 1).is_err(), "an unknown key");
        add_text(&conn, "qaa", key).unwrap();
        assert!(
            add_text(&conn, "qaa", key).is_err(),
            "one text per language and key"
        );
    }

    #[test]
    fn deleting_a_key_or_a_language_deletes_only_its_texts() {
        let conn = migrated_connection();
        let languages = ["qaa", "qab"];
        for code in languages {
            add_language(&conn, code, false, true).unwrap();
        }
        let title = add_key(&conn, "frontend", "test", "title").unwrap();
        let body = add_key(&conn, "frontend", "test", "body").unwrap();
        for code in languages {
            for key in [title, body] {
                add_text(&conn, code, key).unwrap();
            }
        }

        conn.execute("DELETE FROM translation_keys WHERE id = ?1", [title])
            .unwrap();
        conn.execute("DELETE FROM locales WHERE code = 'qab'", [])
            .unwrap();

        let left: Vec<(String, i64)> = conn
            .prepare("SELECT locale_code, key_id FROM translations")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(left, [("qaa".to_string(), body)]);
    }

    #[test]
    fn a_setting_names_an_existing_language_and_loses_it_with_the_language() {
        let conn = migrated_connection();
        let alice = insert_user(&conn, "alice");
        let bob = insert_user(&conn, "bob");
        add_language(&conn, "qaa", false, true).unwrap();
        add_language(&conn, "qab", false, true).unwrap();
        let set = |user_id: i64, code: &str| {
            conn.execute(
                "INSERT INTO user_settings (user_id, locale) VALUES (?1, ?2)",
                params![user_id, code],
            )
        };

        assert!(set(alice, "qzz").is_err(), "a language that doesn't exist");
        set(alice, "qaa").unwrap();
        set(bob, "qab").unwrap();

        conn.execute("DELETE FROM locales WHERE code = 'qaa'", [])
            .unwrap();

        let settings: Vec<(i64, Option<String>)> = conn
            .prepare("SELECT user_id, locale FROM user_settings ORDER BY user_id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            settings,
            [(alice, None), (bob, Some("qab".to_string()))],
            "alice keeps her row, now on the default; bob keeps his language"
        );
    }

    #[test]
    fn running_the_migrations_again_changes_nothing() {
        let mut conn = memory_connection();
        run(&mut conn).unwrap();

        let applied: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(applied, MIGRATIONS.len() as i64);
    }

    /// A book with every column filled, as a scan of a CBZ with a ComicInfo.xml stores one.
    fn insert_full_book(
        conn: &Connection,
        file_path: &str,
        series: Option<i64>,
    ) -> rusqlite::Result<i64> {
        conn.execute(
            "INSERT INTO books (title, author, format, language, cover_image, description, tags, \
             file_path, page_count, volume_number, chapter_number, page_number, file_hash, \
             last_modified, file_size, series, reading_direction) \
             VALUES ('Berserk', 'Kentaro Miura', 'cbz', 'ja', 'abc.jpg', 'Dark fantasy', \
             'manga,dark', ?1, 224, 1, 3, NULL, 'abc', 1700000000, 52428800, ?2, 'rtl')",
            params![file_path, series],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Every row of `books` in id order, each column as stored.
    fn book_rows(conn: &Connection) -> Vec<Vec<rusqlite::types::Value>> {
        let mut stmt = conn.prepare("SELECT * FROM books ORDER BY id").unwrap();
        let columns = stmt.column_count();
        stmt.query_map([], |row| (0..columns).map(|i| row.get(i)).collect())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn a_database_of_the_second_schema_keeps_every_book_and_id_as_series_becomes_a_foreign_key() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        // As a server of the second schema left it.
        conn.execute_batch(MIGRATIONS[0].sql).unwrap();
        conn.execute_batch(MIGRATIONS[1].sql).unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, \
             applied_at INTEGER NOT NULL); \
             INSERT INTO schema_migrations (version, applied_at) VALUES (1, 0), (2, 0);",
        )
        .unwrap();
        let berserk = insert_series(&conn, "berserk");
        insert_full_book(&conn, "/books/Berserk v01.cbz", Some(berserk)).unwrap();
        // Without a declared key, nothing stopped a book from naming a series that doesn't exist.
        let dangling =
            insert_full_book(&conn, "/books/Berserk v02.cbz", Some(berserk + 41)).unwrap();
        insert_full_book(&conn, "/books/Loose.cbz", None).unwrap();
        let top = insert_full_book(&conn, "/books/Berserk v03.cbz", Some(berserk)).unwrap();
        conn.execute("DELETE FROM books WHERE id = ?1", [top])
            .unwrap();
        let before = book_rows(&conn);

        run(&mut conn).unwrap();

        let series_column = conn
            .prepare("SELECT * FROM books")
            .unwrap()
            .column_index("series")
            .unwrap();
        let dangling_row = before
            .iter()
            .position(|row| row[0] == rusqlite::types::Value::Integer(dangling))
            .unwrap();
        let mut expected = before;
        expected[dangling_row][series_column] = rusqlite::types::Value::Null;
        // Later migrations append columns; the ones this schema had come first.
        let columns = expected[0].len();
        let kept: Vec<_> = book_rows(&conn)
            .into_iter()
            .map(|row| row[..columns].to_vec())
            .collect();
        assert_eq!(kept, expected, "every book, id and column kept");

        let next = insert_full_book(&conn, "/books/Berserk v04.cbz", Some(berserk)).unwrap();
        assert_eq!(next, top + 1, "the deleted top id is not handed out again");

        assert!(
            insert_full_book(&conn, "/books/Ghost.cbz", Some(berserk + 41)).is_err(),
            "a book names an existing series or none"
        );
        assert!(
            insert_full_book(&conn, "/books/Berserk v01.cbz", Some(berserk)).is_err(),
            "file paths stay unique"
        );
        let unknown_direction = conn.execute(
            "INSERT INTO books (title, author, format, language, tags, file_path, \
             reading_direction) VALUES ('t', 'a', 'cbz', 'ja', '', '/books/up.cbz', 'up')",
            [],
        );
        assert!(unknown_direction.is_err(), "the direction's CHECK stays");

        let index: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' \
                 AND tbl_name = 'books' AND name = 'books_by_series'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index, 1);
        let mut check = conn.prepare("PRAGMA foreign_key_check").unwrap();
        assert!(check.query([]).unwrap().next().unwrap().is_none());
        let foreign_keys: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1, "the migration leaves foreign keys on");
    }

    #[test]
    fn a_database_of_the_third_schema_keeps_cover_file_names_and_loses_the_shared_broken_one() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        // As a server of the third schema left it.
        for migration in &MIGRATIONS[..3] {
            conn.execute_batch(migration.sql).unwrap();
        }
        conn.execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, \
             applied_at INTEGER NOT NULL); \
             INSERT INTO schema_migrations (version, applied_at) VALUES (1, 0), (2, 0), (3, 0);",
        )
        .unwrap();
        let book = |path: &str, cover: Option<&str>| {
            conn.execute(
                "INSERT INTO books (title, author, format, language, tags, file_path, \
                 cover_image) VALUES ('t', 'a', 'cbz', 'ja', '', ?1, ?2)",
                params![path, cover],
            )
            .unwrap();
        };
        book("/books/a.cbz", Some("./cover_images/abc.jpg"));
        book("/books/b.cbz", Some("./cover_images.jpg"));
        book("/books/c.cbz", Some("./cover_images.png"));
        book("/books/d.cbz", None);
        book("/books/e.cbz", Some("def.png"));
        conn.execute(
            "INSERT INTO series (name, cover_image) VALUES ('s', './cover_images/s.webp')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO library (library_name, library_type, cover_image) \
             VALUES ('l', 'books', './cover_images/l.gif')",
            [],
        )
        .unwrap();

        run(&mut conn).unwrap();

        let covers = |sql: &str| -> Vec<Option<String>> {
            conn.prepare(sql)
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(
            covers("SELECT cover_image FROM books ORDER BY file_path"),
            [
                Some("abc.jpg".to_string()),
                None,
                None,
                None,
                Some("def.png".to_string())
            ]
        );
        assert_eq!(
            covers("SELECT cover_image FROM series"),
            [Some("s.webp".to_string())]
        );
        assert_eq!(
            covers("SELECT cover_image FROM library"),
            [Some("l.gif".to_string())]
        );
    }

    #[test]
    fn a_database_of_the_fourth_schema_gets_match_keys_and_its_scanned_series_their_capitals_back()
    {
        let mut conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        // As a server of the fourth schema left it.
        for migration in &MIGRATIONS[..4] {
            conn.execute_batch(migration.sql).unwrap();
        }
        conn.execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, \
             applied_at INTEGER NOT NULL); \
             INSERT INTO schema_migrations (version, applied_at) \
             VALUES (1, 0), (2, 0), (3, 0), (4, 0);",
        )
        .unwrap();
        let book = |title: &str, series: i64| {
            conn.execute(
                "INSERT INTO books (title, author, format, language, tags, file_path, series) \
                 VALUES (?1, 'a', 'cbz', 'ja', '', ?2, ?3)",
                params![title, format!("/books/{title} {series}.cbz"), series],
            )
            .unwrap();
        };
        // As the scanner named them: its first book's title, lowercased in Rust.
        let berserk = insert_series(&conn, "berserk");
        book("Berserk", berserk);
        book("BERSERK", berserk);
        let cafe = insert_series(&conn, "café");
        book("Café", cafe);
        let anger = insert_series(&conn, "ärger");
        book("Ärger", anger);
        // As an admin renamed one through the API, and one without books.
        let pluto = insert_series(&conn, "Pluto");
        book("Pluto", pluto);
        insert_series(&conn, "empty");

        run(&mut conn).unwrap();

        let rows: Vec<(String, String)> = conn
            .prepare("SELECT name, match_key FROM series ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let expected = [
            // The first book's capitals.
            ("Berserk", "berserk"),
            // lower() folds the C; the é was lowercase already.
            ("Café", "café"),
            // lower() can't fold Ä, so no title lowers to the name, which stays.
            ("ärger", "ärger"),
            // An admin's capitals stay, and the key is lowercase.
            ("Pluto", "pluto"),
            ("empty", "empty"),
        ]
        .map(|(name, key)| (name.to_string(), key.to_string()));
        assert_eq!(rows, expected);

        let index: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' \
                 AND tbl_name = 'series' AND name = 'series_by_match_key'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index, 1);
    }

    #[test]
    fn only_a_series_without_books_can_be_deleted_and_its_reading_modes_go_with_it() {
        let conn = memory_connection();
        let alice = insert_user(&conn, "alice");
        let berserk = insert_series(&conn, "berserk");
        let monster = insert_series(&conn, "monster");
        insert_full_book(&conn, "/books/Berserk v01.cbz", Some(berserk)).unwrap();
        for series_id in [berserk, monster] {
            add_reading_mode(&conn, alice, series_id, "rtl").unwrap();
        }

        let refused = conn.execute("DELETE FROM series WHERE id = ?1", [berserk]);
        assert!(refused.is_err(), "a series with books stays");
        conn.execute("DELETE FROM series WHERE id = ?1", [monster])
            .unwrap();

        let left: Vec<i64> = conn
            .prepare("SELECT series_id FROM reading_modes")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(left, [berserk]);
    }

    #[test]
    fn a_database_of_the_fifth_schema_keeps_every_book_outside_any_directory() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        // As a server of the fifth schema left it.
        for migration in &MIGRATIONS[..5] {
            conn.execute_batch(migration.sql).unwrap();
        }
        conn.execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, \
             applied_at INTEGER NOT NULL); \
             INSERT INTO schema_migrations (version, applied_at) \
             VALUES (1, 0), (2, 0), (3, 0), (4, 0), (5, 0);",
        )
        .unwrap();
        let berserk = insert_series(&conn, "berserk");
        insert_full_book(&conn, "/books/Berserk v01.cbz", Some(berserk)).unwrap();
        insert_full_book(&conn, "../Downloads/Loose.cbz", None).unwrap();
        let before = book_rows(&conn);

        run(&mut conn).unwrap();

        let after = book_rows(&conn);
        assert_eq!(after.len(), before.len());
        for (old, new) in before.iter().zip(&after) {
            assert_eq!(new[..old.len()], old[..], "every column kept");
            // Later migrations append columns of their own after it.
            assert_eq!(
                new[old.len()],
                rusqlite::types::Value::Null,
                "and in no directory"
            );
        }
        let index: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' \
                 AND tbl_name = 'books' AND name = 'books_by_directory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index, 1);
        let mut check = conn.prepare("PRAGMA foreign_key_check").unwrap();
        assert!(check.query([]).unwrap().next().unwrap().is_none());
    }

    fn insert_directory(
        conn: &Connection,
        path: &str,
        library_id: Option<i64>,
    ) -> rusqlite::Result<i64> {
        conn.execute(
            "INSERT INTO directories (path, library_id) VALUES (?1, ?2)",
            params![path, library_id],
        )?;
        Ok(conn.last_insert_rowid())
    }

    #[test]
    fn a_database_of_the_sixth_schema_keeps_its_books_with_no_fingerprint_yet() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        // As a server of the sixth schema left it.
        for migration in &MIGRATIONS[..6] {
            conn.execute_batch(migration.sql).unwrap();
        }
        conn.execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, \
             applied_at INTEGER NOT NULL); \
             INSERT INTO schema_migrations (version, applied_at) \
             VALUES (1, 0), (2, 0), (3, 0), (4, 0), (5, 0), (6, 0);",
        )
        .unwrap();
        let berserk = insert_series(&conn, "berserk");
        insert_full_book(&conn, "/books/Berserk v01.cbz", Some(berserk)).unwrap();
        let before = book_rows(&conn);

        run(&mut conn).unwrap();

        let after = book_rows(&conn);
        assert_eq!(after.len(), 1);
        assert_eq!(
            after[0][..before[0].len()],
            before[0][..],
            "every column kept"
        );
        assert_eq!(
            after[0][before[0].len()],
            rusqlite::types::Value::Null,
            "no fingerprint until its file is read again"
        );
        let index: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' \
                 AND tbl_name = 'books' AND name = 'books_by_hash'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index, 1);
    }

    /// A database as a server of the seventh schema left it.
    fn seventh_schema() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        for migration in &MIGRATIONS[..7] {
            conn.execute_batch(migration.sql).unwrap();
        }
        conn.execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, \
             applied_at INTEGER NOT NULL); \
             INSERT INTO schema_migrations (version, applied_at) \
             VALUES (1, 0), (2, 0), (3, 0), (4, 0), (5, 0), (6, 0), (7, 0);",
        )
        .unwrap();
        conn
    }

    fn series_names_and_keys(conn: &Connection) -> Vec<(String, String)> {
        conn.prepare("SELECT name, match_key FROM series ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn applied(conn: &Connection, version: i64) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = ?1",
            [version],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
            == 1
    }

    #[test]
    fn a_database_of_the_seventh_schema_gets_its_match_keys_composed() {
        let mut conn = seventh_schema();
        let series = |name: &str, key: &str| {
            conn.execute(
                "INSERT INTO series (name, match_key) VALUES (?1, ?2)",
                params![name, key],
            )
            .unwrap();
        };
        // "Ä" as "A" and a combining diaeresis, as older macOS disks store it; then as one character.
        series("A\u{308}rger", "a\u{308}rger");
        series("\u{c4}rger", "\u{e4}rger");
        series("Berserk", "berserk");

        run(&mut conn).unwrap();

        let expected = [
            ("A\u{308}rger", "\u{e4}rger"),
            ("\u{c4}rger", "\u{e4}rger"),
            ("Berserk", "berserk"),
        ]
        .map(|(name, key)| (name.to_string(), key.to_string()));
        assert_eq!(
            series_names_and_keys(&conn),
            expected,
            "only the keys composed, the names as they were"
        );
        assert!(applied(&conn, 8));
    }

    #[test]
    fn a_migration_whose_step_fails_is_undone_whole_and_not_recorded() {
        let mut conn = seventh_schema();
        conn.execute_batch(
            "INSERT INTO series (name, match_key) VALUES \
             ('A\u{308}rger', 'a\u{308}rger'), ('O\u{308}l', 'o\u{308}l'); \
             CREATE TRIGGER refuse_the_second BEFORE UPDATE ON series WHEN NEW.id = 2 \
             BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();

        assert!(run(&mut conn).is_err());

        let expected = [("A\u{308}rger", "a\u{308}rger"), ("O\u{308}l", "o\u{308}l")]
            .map(|(name, key)| (name.to_string(), key.to_string()));
        assert_eq!(
            series_names_and_keys(&conn),
            expected,
            "the first key, composed before the second failed, is back as it was"
        );
        assert!(!applied(&conn, 8));
    }

    #[test]
    fn a_directory_is_added_once_and_names_an_existing_library_or_none() {
        let conn = memory_connection();
        insert_directory(&conn, "/books", None).unwrap();

        assert!(
            insert_directory(&conn, "/books", None).is_err(),
            "one row per path"
        );
        assert!(
            insert_directory(&conn, "/comics", Some(999)).is_err(),
            "only a library that exists"
        );
        let created_at: i64 = conn
            .query_row(
                "SELECT created_at FROM directories WHERE path = '/books'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(created_at > 1_700_000_000, "stamped when it was added");
    }

    #[test]
    fn removing_a_directory_removes_its_books_and_deleting_its_library_only_unlinks_it() {
        let conn = memory_connection();
        conn.execute(
            "INSERT INTO library (library_name, library_type) VALUES ('Manga', 'comics')",
            [],
        )
        .unwrap();
        let library = conn.last_insert_rowid();
        let manga = insert_directory(&conn, "/manga", Some(library)).unwrap();
        let comics = insert_directory(&conn, "/comics", None).unwrap();
        let place = |path: &str, directory: i64| {
            let book = insert_full_book(&conn, path, None).unwrap();
            conn.execute(
                "UPDATE books SET directory_id = ?1 WHERE id = ?2",
                [directory, book],
            )
            .unwrap();
        };
        place("/manga/Berserk v01.cbz", manga);
        place("/manga/Berserk v02.cbz", manga);
        place("/comics/Saga v01.cbz", comics);
        insert_full_book(&conn, "/old/Loose.cbz", None).unwrap();
        assert!(
            conn.execute(
                "UPDATE books SET directory_id = 999 WHERE directory_id IS NULL",
                []
            )
            .is_err(),
            "a book names an existing directory or none"
        );

        conn.execute("DELETE FROM library WHERE id = ?1", [library])
            .unwrap();
        let libraries: Vec<Option<i64>> = conn
            .prepare("SELECT library_id FROM directories ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(libraries, [None, None], "both directories stay");

        conn.execute("DELETE FROM directories WHERE id = ?1", [manga])
            .unwrap();
        let left: Vec<String> = conn
            .prepare("SELECT file_path FROM books ORDER BY file_path")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(left, ["/comics/Saga v01.cbz", "/old/Loose.cbz"]);
    }

    #[test]
    fn a_series_holds_an_extra_key_once_and_its_keys_go_with_it() {
        let conn = memory_connection();
        let berserk = insert_series(&conn, "berserk");
        let monster = insert_series(&conn, "monster");
        let add_key = |series_id: i64, key: &str| {
            conn.execute(
                "INSERT INTO series_match_keys (series_id, match_key) VALUES (?1, ?2)",
                params![series_id, key],
            )
        };

        add_key(berserk, "berserk deluxe").unwrap();
        // Two series may hold one key: each is found by it only in its own directory.
        add_key(monster, "berserk deluxe").unwrap();
        assert!(
            add_key(berserk, "berserk deluxe").is_err(),
            "one row per series and key"
        );
        assert!(
            add_key(monster + 1, "monster").is_err(),
            "only a series that exists"
        );

        conn.execute("DELETE FROM series WHERE id = ?1", [berserk])
            .unwrap();
        let left: Vec<(i64, String)> = conn
            .prepare("SELECT series_id, match_key FROM series_match_keys ORDER BY series_id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(left, [(monster, "berserk deluxe".to_string())]);
    }
}
