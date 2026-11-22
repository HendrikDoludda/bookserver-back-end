use std::net::SocketAddr;

use axum::{
    routing::{delete, get, patch, post, put},
    Router,
};
use tokio::net::TcpListener;
use tracing::info;

use crate::{
    app_state::AppState,
    error_types::RoutingErrors,
    routes::api_caller::{
        account::{
            change_email, change_password, change_username, confirm_email_change, get_account,
        },
        authentication::{
            confirm_password_reset, log_out, refresh_session, request_password_reset,
            resend_verification_code, sign_in, sign_up, verify_email,
        },
        directories::{add_directory, list_directories, remove_directory},
        helpers::{method_not_allowed, not_found, not_implemented, ping_server},
        library::{
            add_library, add_series_to_library, choose_library_cover, choose_series_cover,
            delete_library, delete_series, get_cover, get_libraries, get_page, get_pages,
            get_series_children, get_series_in_library, list_series, remove_series_from_library,
            request_file, reset_library_cover, reset_series_cover, retrieve_metadata,
            scan_all_directories, set_reading_mode, update_library, update_series,
        },
        locales::{get_translations, list_locales},
        settings::{get_settings, replace_settings},
        totp::{
            disable_totp, finalize_totp_set_up, renew_totp_recovery_codes, sign_in_totp,
            sign_in_totp_with_recovery_code, totp_start_set_up, totp_status,
        },
        users::{list_users, update_user},
    },
};

/// Binds to `address` and serves the API until the process stops.
pub async fn start_server(state: AppState, address: SocketAddr) -> Result<(), RoutingErrors> {
    let app = set_up_routes().with_state(state);

    info!("Starting server on {address}!");

    let listener = TcpListener::bind(address)
        .await
        .map_err(RoutingErrors::TcpListenerCreationFailed)?;

    axum::serve(listener, app)
        .await
        .map_err(RoutingErrors::AxumInitializationFailed)?;
    Ok(())
}

/// The route table. Everything except `/health` lives under `/api/v1`.
/// Routes mapped to `not_implemented` are placeholders whose handlers don't exist yet.
pub fn set_up_routes() -> Router<AppState> {
    let api = Router::new()
        .route("/libraries", get(get_libraries).post(add_library))
        .route(
            "/libraries/{id}",
            delete(delete_library).patch(update_library),
        )
        .route("/libraries/{id}/series", get(get_series_in_library))
        .route(
            "/libraries/{id}/series/{series_id}",
            put(add_series_to_library).delete(remove_series_from_library),
        )
        .route(
            "/libraries/{id}/cover",
            put(choose_library_cover).delete(reset_library_cover),
        )
        .route("/series", get(list_series))
        .route("/series/{id}", patch(update_series).delete(delete_series))
        .route("/series/{id}/books", get(get_series_children))
        .route(
            "/series/{id}/cover",
            put(choose_series_cover).delete(reset_series_cover),
        )
        .route("/series/{id}/reading_mode", put(set_reading_mode))
        .route("/books/{id}", get(retrieve_metadata))
        .route("/books/{id}/file", get(request_file))
        .route("/books/{id}/pages", get(get_pages))
        .route("/books/{id}/pages/{number}", get(get_page))
        .route(
            "/books/{id}/progress",
            get(not_implemented).patch(not_implemented),
        )
        .route("/covers/{kind}/{id}", get(get_cover))
        .route("/scan", post(scan_all_directories))
        .route("/scan/status", get(not_implemented))
        .route("/directories", get(list_directories).post(add_directory))
        .route("/directories/{id}", delete(remove_directory))
        .route("/directories/{id}/scan", post(not_implemented))
        .route("/metadata/scan", post(not_implemented))
        .route("/search", get(not_implemented))
        .route("/auth/register", post(sign_up))
        .route("/auth/verify_email", post(verify_email))
        .route("/auth/verify_email/resend", post(resend_verification_code))
        .route("/auth/login", post(sign_in))
        .route("/auth/login/totp", post(sign_in_totp))
        .route(
            "/auth/login/recovery_code",
            post(sign_in_totp_with_recovery_code),
        )
        .route("/auth/logout", post(log_out))
        .route("/auth/refresh", post(refresh_session))
        .route("/auth/password_reset", post(request_password_reset))
        .route("/auth/password_reset/confirm", post(confirm_password_reset))
        .route("/account", get(get_account))
        .route("/account/password", put(change_password))
        .route("/account/username", put(change_username))
        .route("/account/email", put(change_email))
        .route("/account/email/confirm", post(confirm_email_change))
        .route("/account/settings", get(get_settings).put(replace_settings))
        .route("/account/totp", get(totp_status).post(totp_start_set_up))
        .route("/account/totp/confirm", post(finalize_totp_set_up))
        .route("/account/totp/disable", post(disable_totp))
        .route(
            "/account/totp/recovery_codes",
            post(renew_totp_recovery_codes),
        )
        .route("/users", get(list_users))
        .route("/users/{id}", patch(update_user))
        .route("/locales", get(list_locales))
        .route("/locales/{code}/translations", get(get_translations));

    Router::new()
        .route("/health", get(ping_server))
        .nest("/api/v1", api)
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::set_up_routes;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use axum::{http::HeaderMap, Router};

    use crate::{
        routes::{
            auth::{hash_secret, hash_token},
            email_helper::Mailer,
        },
        test_support::{
            code_in, encoded_image, insert_texts, insert_user, TempDatabase, TempDir,
            TEST_SETUP_CODE,
        },
    };

    #[tokio::test]
    async fn health_answers_pong_through_the_router() {
        let temp = TempDatabase::open();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        let response = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"pong");
    }

    async fn json_answer(request: Request<Body>) -> (StatusCode, serde_json::Value) {
        let temp = TempDatabase::open();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn a_guarded_route_without_a_session_answers_401_json() {
        let (status, body) = json_answer(guarded(None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body, serde_json::json!({ "error": "unauthorized" }));
    }

    #[tokio::test]
    async fn a_missing_book_answers_404_json() {
        let temp = TempDatabase::open();
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        let (status, _, body) =
            call(&app, authorized("GET", "/api/v1/books/999", &member, None)).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json(&body), serde_json::json!({ "error": "not_found" }));
    }

    fn bare(method: &str, uri: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    /// A new user with this role (`"admin"` or `"member"`) and a live session; answers the
    /// session token.
    fn logged_in(temp: &TempDatabase, username: &str, role: &str) -> String {
        let user = temp
            .db
            .setup_new_transaction(|conn| {
                let user = insert_user(conn, username);
                conn.execute(
                    "UPDATE users SET role = ?1 WHERE id = ?2",
                    rusqlite::params![role, user],
                )
                .unwrap();
                Ok(user)
            })
            .unwrap();
        let token = format!("{username}-session");
        add_session(
            temp,
            user,
            "device",
            &token,
            &format!("{username}-refresh"),
            unix_now() + 600,
        );
        token
    }

    #[tokio::test]
    async fn every_route_answers_on_its_new_path_and_the_old_paths_are_gone() {
        let temp = TempDatabase::open();
        let member = logged_in(&temp, "bob", "member");
        let admin = logged_in(&temp, "alice", "admin");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        for uri in [
            "/api/v1/libraries",
            "/api/v1/libraries/1/series",
            "/api/v1/series/1/books",
        ] {
            let (status, _, body) = call(&app, authorized("GET", uri, &member, None)).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert_eq!(json(&body), serde_json::json!([]), "{uri}");
        }
        for uri in ["/api/v1/books/1", "/api/v1/books/1/file"] {
            let (status, _, body) = call(&app, authorized("GET", uri, &member, None)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(
                json(&body),
                serde_json::json!({ "error": "not_found" }),
                "{uri}"
            );
        }

        let library = temp
            .db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO library (library_name, library_type) VALUES ('books', 'books')",
                    [],
                )
                .unwrap();
                Ok(conn.last_insert_rowid())
            })
            .unwrap();
        let uri = format!("/api/v1/libraries/{library}");
        let (status, _, body) = call(&app, authorized("DELETE", &uri, &admin, None)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_empty());

        // A GET proves the path exists without starting a scan.
        let (status, _, _) = call(&app, bare("GET", "/api/v1/scan")).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);

        for (method, uri) in [
            ("GET", "/api/v1/get_libraries"),
            ("GET", "/api/v1/get_series_from_library/1"),
            ("GET", "/api/v1/get_series_children/1"),
            ("GET", "/api/v1/book_metadata/1"),
            ("GET", "/api/v1/book/1"),
            ("POST", "/api/v1/scan_all_directories"),
            ("POST", "/api/v1/setup_db"),
            ("DELETE", "/api/v1/delete_library/1"),
            ("PUT", "/api/v1/update_entry"),
            ("POST", "/api/v1/insert_entry"),
            ("POST", "/api/v1/scan_metadata"),
            ("GET", "/api/v1/reading_progress/1"),
            ("POST", "/api/v1/scan_directory/1"),
            ("GET", "/api/v1/get_cover/book/1"),
        ] {
            let (status, _, _) = call(&app, bare(method, uri)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn every_unbuilt_route_answers_501_json() {
        let temp = TempDatabase::open();
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        for (method, uri) in [
            ("GET", "/api/v1/books/1/progress"),
            ("PATCH", "/api/v1/books/1/progress"),
            ("GET", "/api/v1/scan/status"),
            ("POST", "/api/v1/directories/1/scan"),
            ("POST", "/api/v1/metadata/scan"),
            ("GET", "/api/v1/search"),
        ] {
            let (status, _, body) = call(&app, authorized(method, uri, &member, None)).await;
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{method} {uri}");
            assert_eq!(
                json(&body),
                serde_json::json!({ "error": "not_implemented" }),
                "{method} {uri}"
            );
        }
    }

    #[tokio::test]
    async fn an_unknown_path_answers_404_json_and_a_wrong_method_405_json() {
        let temp = TempDatabase::open();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        for uri in ["/nope", "/api/v1/nope", "/api/v1/books/1/nope"] {
            let (status, _, body) = call(&app, bare("GET", uri)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(
                json(&body),
                serde_json::json!({ "error": "not_found" }),
                "{uri}"
            );
        }

        for (method, uri) in [
            ("GET", "/api/v1/scan"),
            ("DELETE", "/health"),
            ("PUT", "/api/v1/auth/login"),
        ] {
            let (status, headers, body) = call(&app, bare(method, uri)).await;
            assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {uri}");
            assert_eq!(
                json(&body),
                serde_json::json!({ "error": "method_not_allowed" }),
                "{method} {uri}"
            );
            assert!(
                headers.contains_key("allow"),
                "a 405 names the allowed methods: {method} {uri}"
            );
        }
    }

    #[tokio::test]
    async fn the_library_and_series_lists_load_what_the_scanner_wrote() {
        let temp = TempDatabase::open();
        let (library, series) = temp
            .db
            .setup_new_transaction(|conn| {
                // Like the scanner: no covers, descriptions or release years.
                conn.execute(
                    "INSERT INTO library (library_name, library_type) VALUES ('/books', 'books')",
                    [],
                )
                .unwrap();
                let library = conn.last_insert_rowid();
                conn.execute("INSERT INTO series (name) VALUES ('the long road')", [])
                    .unwrap();
                let series = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO library_elements (library_id, series_id) VALUES (?1, ?2)",
                    [library, series],
                )
                .unwrap();
                Ok((library, series))
            })
            .unwrap();
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        let (status, _, body) =
            call(&app, authorized("GET", "/api/v1/libraries", &member, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json(&body),
            serde_json::json!([{
                "id": library,
                "name": "/books",
                "library_type": "books",
                "description": null,
                "series_count": 1,
            }])
        );

        let uri = format!("/api/v1/libraries/{library}/series");
        let (status, _, body) = call(&app, authorized("GET", &uri, &member, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json(&body),
            serde_json::json!([{
                "id": series,
                "name": "the long road",
                "description": null,
                "start_release_year": null,
                "end_release_year": null,
                "book_count": 0,
            }])
        );
    }

    #[tokio::test]
    async fn the_library_and_series_lists_never_send_a_stored_cover_path() {
        let temp = TempDatabase::open();
        let library = temp
            .db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO library (library_name, library_type, cover_image) \
                     VALUES ('manga', 'comics', '/srv/covers/manga.jpg')",
                    [],
                )
                .unwrap();
                let library = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO series (name, cover_image) VALUES ('berserk', '/srv/covers/b.jpg')",
                    [],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO library_elements (library_id, series_id) VALUES (?1, ?2)",
                    [library, conn.last_insert_rowid()],
                )
                .unwrap();
                Ok(library)
            })
            .unwrap();
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        for uri in [
            "/api/v1/libraries".to_string(),
            format!("/api/v1/libraries/{library}/series"),
        ] {
            let (status, _, body) = call(&app, authorized("GET", &uri, &member, None)).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            let row = &json(&body)[0];
            assert!(row.get("cover_image").is_none(), "{uri}: {row}");
            assert!(!row.to_string().contains("/srv/covers"), "{uri}: {row}");
        }
    }

    /// A series with plain SQL, as the scanner leaves one; answers its id.
    fn scanned_series(temp: &TempDatabase, name: &str) -> i64 {
        temp.db
            .setup_new_transaction(|conn| {
                conn.execute("INSERT INTO series (name) VALUES (?1)", [name])
                    .unwrap();
                Ok(conn.last_insert_rowid())
            })
            .unwrap()
    }

    #[tokio::test]
    async fn an_admin_creates_a_library_edits_it_and_chooses_its_series() {
        let temp = TempDatabase::open();
        let admin = logged_in(&temp, "alice", "admin");
        let berserk = scanned_series(&temp, "berserk");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        let body = serde_json::json!({
            "name": " Favourites ",
            "library_type": "comics",
            "description": "Picked by hand",
        });
        let (status, _, created) = call(
            &app,
            authorized("POST", "/api/v1/libraries", &admin, Some(body)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let library = json(&created)["id"].as_i64().unwrap();
        assert_eq!(
            json(&created),
            serde_json::json!({
                "id": library,
                "name": "Favourites",
                "library_type": "comics",
                "description": "Picked by hand",
                "series_count": 0,
            })
        );

        let uri = format!("/api/v1/libraries/{library}");
        let body = serde_json::json!({ "library_type": "books", "description": null });
        let (status, _, changed) = call(&app, authorized("PATCH", &uri, &admin, Some(body))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json(&changed),
            serde_json::json!({
                "id": library,
                "name": "Favourites",
                "library_type": "books",
                "description": null,
                "series_count": 0,
            })
        );

        let membership = format!("/api/v1/libraries/{library}/series/{berserk}");
        for _ in 0..2 {
            let (status, _, body) = call(&app, authorized("PUT", &membership, &admin, None)).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            assert!(body.is_empty());
        }
        let (status, _, listed) =
            call(&app, authorized("GET", "/api/v1/series", &admin, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json(&listed),
            serde_json::json!([{
                "id": berserk,
                "name": "berserk",
                "description": null,
                "start_release_year": null,
                "end_release_year": null,
                "book_count": 0,
                "library_ids": [library],
            }])
        );
        let (_, _, in_library) = call(
            &app,
            authorized("GET", &format!("{uri}/series"), &admin, None),
        )
        .await;
        assert_eq!(json(&in_library)[0]["id"], berserk);

        let (status, _, _) = call(&app, authorized("DELETE", &membership, &admin, None)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, _, listed) = call(&app, authorized("GET", "/api/v1/series", &admin, None)).await;
        assert_eq!(json(&listed)[0]["library_ids"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn a_library_with_an_unknown_type_a_blank_name_or_no_change_is_refused() {
        let temp = TempDatabase::open();
        let admin = logged_in(&temp, "alice", "admin");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let create =
            |body: serde_json::Value| authorized("POST", "/api/v1/libraries", &admin, Some(body));

        let (status, _, body) = call(
            &app,
            create(serde_json::json!({ "name": "Novels", "library_type": "novels" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json(&body),
            serde_json::json!({ "error": "invalid_request" })
        );

        let (status, _, body) = call(
            &app,
            create(serde_json::json!({ "name": "  ", "library_type": "books" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json(&body), serde_json::json!({ "error": "invalid_name" }));

        let (_, _, created) = call(
            &app,
            create(serde_json::json!({ "name": "Manga", "library_type": "comics" })),
        )
        .await;
        let library = json(&created)["id"].as_i64().unwrap();
        let (status, _, body) = call(
            &app,
            authorized(
                "PATCH",
                &format!("/api/v1/libraries/{library}"),
                &admin,
                Some(serde_json::json!({})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json(&body),
            serde_json::json!({ "error": "invalid_request" })
        );

        for (method, uri) in [
            ("PATCH", format!("/api/v1/libraries/{}", library + 1)),
            ("PUT", format!("/api/v1/libraries/{library}/series/999")),
        ] {
            let body = (method == "PATCH").then(|| serde_json::json!({ "name": "Comics" }));
            let (status, _, answer) = call(&app, authorized(method, &uri, &admin, body)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
            assert_eq!(json(&answer), serde_json::json!({ "error": "not_found" }));
        }
    }

    #[tokio::test]
    async fn only_an_admin_creates_or_changes_libraries_and_lists_every_series() {
        let temp = TempDatabase::open();
        let member = logged_in(&temp, "bob", "member");
        let berserk = scanned_series(&temp, "berserk");
        let library = temp
            .db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO library (library_name, library_type) VALUES ('manga', 'comics')",
                    [],
                )
                .unwrap();
                Ok(conn.last_insert_rowid())
            })
            .unwrap();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let membership = format!("/api/v1/libraries/{library}/series/{berserk}");

        for (method, uri, body) in [
            (
                "POST",
                "/api/v1/libraries".to_string(),
                Some(serde_json::json!({ "name": "Mine", "library_type": "books" })),
            ),
            (
                "PATCH",
                format!("/api/v1/libraries/{library}"),
                Some(serde_json::json!({ "name": "Mine" })),
            ),
            ("GET", "/api/v1/series".to_string(), None),
            ("PUT", membership.clone(), None),
            ("DELETE", membership.clone(), None),
            (
                "PATCH",
                format!("/api/v1/series/{berserk}"),
                Some(serde_json::json!({ "name": "Mine" })),
            ),
            ("DELETE", format!("/api/v1/series/{berserk}"), None),
        ] {
            let (status, _, answer) = call(&app, authorized(method, &uri, &member, body)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
            assert_eq!(json(&answer), serde_json::json!({ "error": "forbidden" }));
        }

        let (names, links): (Vec<String>, i64) = temp
            .db
            .setup_new_transaction(|conn| {
                let names = conn
                    .prepare("SELECT library_name FROM library")
                    .unwrap()
                    .query_map([], |row| row.get(0))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
                let links = conn
                    .query_row("SELECT COUNT(*) FROM library_elements", [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                Ok((names, links))
            })
            .unwrap();
        assert_eq!(names, ["manga"], "a member creates and renames nothing");
        assert_eq!(links, 0, "a member links nothing");
        let series: Vec<String> = temp
            .db
            .setup_new_transaction(|conn| {
                Ok(conn
                    .prepare("SELECT name FROM series")
                    .unwrap()
                    .query_map([], |row| row.get(0))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap())
            })
            .unwrap();
        assert_eq!(
            series,
            ["berserk"],
            "a member renames and deletes no series"
        );
    }

    /// A book of `series` with plain SQL, as the scanner writes one.
    fn scanned_book(temp: &TempDatabase, series: i64, file_path: &str) {
        temp.db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO books (title, author, format, language, tags, file_path, series) \
                     VALUES ('Berserk', 'Unknown Author', 'cbz', 'Unknown', '', ?1, ?2)",
                    rusqlite::params![file_path, series],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
    }

    #[tokio::test]
    async fn an_admin_edits_a_series_and_deletes_it_only_once_it_has_no_books() {
        let temp = TempDatabase::open();
        let admin = logged_in(&temp, "alice", "admin");
        let berserk = scanned_series(&temp, "berserk");
        let monster = scanned_series(&temp, "monster");
        scanned_book(&temp, berserk, "/books/Berserk v01.cbz");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let berserk_uri = format!("/api/v1/series/{berserk}");
        let monster_uri = format!("/api/v1/series/{monster}");

        let body = serde_json::json!({ "name": "Berserk", "start_release_year": 1989 });
        let (status, _, changed) =
            call(&app, authorized("PATCH", &berserk_uri, &admin, Some(body))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json(&changed),
            serde_json::json!({
                "id": berserk,
                "name": "Berserk",
                "description": null,
                "start_release_year": 1989,
                "end_release_year": null,
                "book_count": 1,
            })
        );

        let body = serde_json::json!({ "end_release_year": 1980 });
        let (status, _, answer) =
            call(&app, authorized("PATCH", &berserk_uri, &admin, Some(body))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json(&answer),
            serde_json::json!({ "error": "invalid_release_years" })
        );

        let (status, _, answer) =
            call(&app, authorized("DELETE", &berserk_uri, &admin, None)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            json(&answer),
            serde_json::json!({ "error": "series_not_empty" })
        );

        let (status, _, answer) =
            call(&app, authorized("DELETE", &monster_uri, &admin, None)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(answer.is_empty());
        let (status, _, answer) =
            call(&app, authorized("DELETE", &monster_uri, &admin, None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json(&answer), serde_json::json!({ "error": "not_found" }));
    }

    #[tokio::test]
    async fn deleting_a_library_only_unlinks_its_series_and_keeps_their_books() {
        let temp = TempDatabase::open();
        let admin = logged_in(&temp, "alice", "admin");
        let berserk = scanned_series(&temp, "berserk");
        scanned_book(&temp, berserk, "/books/Berserk v01.cbz");
        let library = temp
            .db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO library (library_name, library_type) VALUES ('manga', 'comics')",
                    [],
                )
                .unwrap();
                let library = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO library_elements (library_id, series_id) VALUES (?1, ?2)",
                    [library, berserk],
                )
                .unwrap();
                Ok(library)
            })
            .unwrap();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        let uri = format!("/api/v1/libraries/{library}");
        let (status, _, _) = call(&app, authorized("DELETE", &uri, &admin, None)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (_, _, listed) = call(&app, authorized("GET", "/api/v1/series", &admin, None)).await;
        assert_eq!(json(&listed)[0]["id"], berserk);
        assert_eq!(json(&listed)[0]["book_count"], 1, "its book stays");
        assert_eq!(json(&listed)[0]["library_ids"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn every_user_reads_their_own_account_with_their_role() {
        let temp = TempDatabase::open();
        let member = logged_in(&temp, "bob", "member");
        let admin = logged_in(&temp, "alice", "admin");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        for (token, username, role) in [(&member, "bob", "member"), (&admin, "alice", "admin")] {
            let (status, _, body) =
                call(&app, authorized("GET", "/api/v1/account", token, None)).await;
            assert_eq!(status, StatusCode::OK, "{username}");
            let account = json(&body);
            assert_eq!(account["username"], username);
            assert_eq!(account["email"], format!("{username}@example.com"));
            assert_eq!(account["role"], role);
            assert!(account["id"].is_i64());
            assert!(account.get("password_hash").is_none(), "{account}");
            assert!(
                !String::from_utf8_lossy(&body).contains("not-a-hash"),
                "the stored hash never goes out"
            );
        }
    }

    #[tokio::test]
    async fn deleting_a_library_no_one_has_answers_404_json() {
        let temp = TempDatabase::open();
        let admin = logged_in(&temp, "alice", "admin");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        let (status, _, body) = call(
            &app,
            authorized("DELETE", "/api/v1/libraries/999", &admin, None),
        )
        .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json(&body), serde_json::json!({ "error": "not_found" }));
    }

    /// `path` canonical, as a directory stores it.
    fn canonical(path: &std::path::Path) -> String {
        std::fs::canonicalize(path)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[tokio::test]
    async fn an_admin_adds_a_folder_lists_it_and_removes_it_over_http() {
        let temp = TempDatabase::open();
        let admin = logged_in(&temp, "alice", "admin");
        let files = TempDir::new();
        let file = files.write("Manga/Berserk v01.cbz", b"cbz");
        let berserk = scanned_series(&temp, "Berserk");
        scanned_book(&temp, berserk, &file.to_string_lossy());
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let folder = files.path().join("Manga");

        let body = serde_json::json!({
            "path": folder.to_string_lossy(),
            "create_library": true,
            "library_type": "comics",
        });
        let (status, _, added) = call(
            &app,
            authorized("POST", "/api/v1/directories", &admin, Some(body)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let directory = json(&added)["id"].as_i64().unwrap();
        let library = json(&added)["library_id"].as_i64().unwrap();
        let expected = serde_json::json!([{
            "id": directory,
            "path": canonical(&folder),
            "library_id": library,
        }]);
        assert_eq!(json(&added), expected[0]);

        let (status, _, listed) =
            call(&app, authorized("GET", "/api/v1/directories", &admin, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&listed), expected);
        let (_, _, libraries) =
            call(&app, authorized("GET", "/api/v1/libraries", &admin, None)).await;
        assert_eq!(
            json(&libraries),
            serde_json::json!([{
                "id": library,
                "name": "Manga",
                "library_type": "comics",
                "description": null,
                "series_count": 1,
            }]),
            "the stored book's series joined the folder's library"
        );

        let uri = format!("/api/v1/directories/{directory}");
        let (status, _, body) = call(&app, authorized("DELETE", &uri, &admin, None)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_empty());
        let (_, _, listed) =
            call(&app, authorized("GET", "/api/v1/directories", &admin, None)).await;
        assert_eq!(json(&listed), serde_json::json!([]));
        let (_, _, series) = call(&app, authorized("GET", "/api/v1/series", &admin, None)).await;
        assert_eq!(
            json(&series),
            serde_json::json!([]),
            "its book left, and its emptied series with it"
        );
        assert!(file.exists(), "the file stays");

        let (status, _, body) = call(&app, authorized("DELETE", &uri, &admin, None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json(&body), serde_json::json!({ "error": "not_found" }));
    }

    #[tokio::test]
    async fn adding_a_folder_answers_its_refusals_as_json() {
        let temp = TempDatabase::open();
        let admin = logged_in(&temp, "alice", "admin");
        let files = TempDir::new();
        files.write("Manga/Berserk v01.cbz", b"cbz");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let path = |relative: &str| files.path().join(relative).to_string_lossy().into_owned();
        let (status, _, _) = call(
            &app,
            authorized(
                "POST",
                "/api/v1/directories",
                &admin,
                Some(serde_json::json!({ "path": path("Manga"), "create_library": false })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        for (body, expected_status, key) in [
            (
                serde_json::json!({ "path": path("Missing"), "create_library": false }),
                StatusCode::BAD_REQUEST,
                "directory_not_found",
            ),
            (
                serde_json::json!({ "path": path("Manga/"), "create_library": false }),
                StatusCode::CONFLICT,
                "directory_taken",
            ),
            (
                serde_json::json!({ "path": path(""), "create_library": false }),
                StatusCode::CONFLICT,
                "directory_overlaps",
            ),
            (
                serde_json::json!({ "path": path("Manga"), "create_library": true }),
                StatusCode::BAD_REQUEST,
                "invalid_request",
            ),
            (
                serde_json::json!({ "path": path("Manga") }),
                StatusCode::BAD_REQUEST,
                "invalid_request",
            ),
            (
                serde_json::json!({
                    "path": path("Manga"),
                    "create_library": true,
                    "library_type": "comics",
                    "library_name": " ",
                }),
                StatusCode::BAD_REQUEST,
                "invalid_name",
            ),
        ] {
            let label = body.to_string();
            let (status, _, answer) = call(
                &app,
                authorized("POST", "/api/v1/directories", &admin, Some(body)),
            )
            .await;
            assert_eq!(status, expected_status, "{label}");
            assert_eq!(
                json(&answer),
                serde_json::json!({ "error": key }),
                "{label}"
            );
        }
    }

    #[tokio::test]
    async fn only_an_admin_adds_lists_or_removes_folders() {
        let temp = TempDatabase::open();
        let member = logged_in(&temp, "bob", "member");
        let files = TempDir::new();
        files.write("Manga/Berserk v01.cbz", b"cbz");
        let directory = temp
            .db
            .setup_new_transaction(|conn| {
                conn.execute("INSERT INTO directories (path) VALUES ('/books')", [])
                    .unwrap();
                Ok(conn.last_insert_rowid())
            })
            .unwrap();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        for (method, uri, body) in [
            ("GET", "/api/v1/directories".to_string(), None),
            (
                "POST",
                "/api/v1/directories".to_string(),
                Some(serde_json::json!({
                    "path": files.path().join("Manga").to_string_lossy(),
                    "create_library": false,
                })),
            ),
            ("DELETE", format!("/api/v1/directories/{directory}"), None),
        ] {
            let (status, _, answer) = call(&app, authorized(method, &uri, &member, body)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
            assert_eq!(json(&answer), serde_json::json!({ "error": "forbidden" }));
        }
        let paths: Vec<String> = temp
            .db
            .setup_new_transaction(|conn| {
                Ok(conn
                    .prepare("SELECT path FROM directories")
                    .unwrap()
                    .query_map([], |row| row.get(0))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap())
            })
            .unwrap();
        assert_eq!(paths, ["/books"], "a member adds and removes nothing");
    }

    #[tokio::test]
    async fn the_library_and_series_lists_count_their_series_and_books() {
        let temp = TempDatabase::open();
        let (manga, empty) = temp
            .db
            .setup_new_transaction(|conn| {
                let library = |name: &str| {
                    conn.execute(
                        "INSERT INTO library (library_name, library_type) VALUES (?1, 'books')",
                        [name],
                    )
                    .unwrap();
                    conn.last_insert_rowid()
                };
                let (manga, empty) = (library("/manga"), library("/empty"));
                for (name, books) in [("alpha", 2), ("beta", 1), ("gamma", 0)] {
                    conn.execute("INSERT INTO series (name) VALUES (?1)", [name])
                        .unwrap();
                    let series = conn.last_insert_rowid();
                    conn.execute(
                        "INSERT INTO library_elements (library_id, series_id) VALUES (?1, ?2)",
                        [manga, series],
                    )
                    .unwrap();
                    for volume in 1..=books {
                        conn.execute(
                            "INSERT INTO books (title, author, format, language, tags, \
                             file_path, volume_number, series) \
                             VALUES (?1, 'Unknown Author', 'cbz', 'Unknown', '', ?2, ?3, ?4)",
                            rusqlite::params![
                                name,
                                format!("/manga/{name} v{volume}.cbz"),
                                volume,
                                series
                            ],
                        )
                        .unwrap();
                    }
                }
                Ok((manga, empty))
            })
            .unwrap();
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let pairs = |body: &[u8], key: &str, count: &str| -> Vec<(serde_json::Value, i64)> {
            json(body)
                .as_array()
                .unwrap()
                .iter()
                .map(|row| (row[key].clone(), row[count].as_i64().unwrap()))
                .collect()
        };

        let (status, _, body) =
            call(&app, authorized("GET", "/api/v1/libraries", &member, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            pairs(&body, "id", "series_count"),
            [(manga.into(), 3), (empty.into(), 0)]
        );

        let uri = format!("/api/v1/libraries/{manga}/series");
        let (status, _, body) = call(&app, authorized("GET", &uri, &member, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            pairs(&body, "name", "book_count"),
            [("alpha".into(), 2), ("beta".into(), 1), ("gamma".into(), 0)]
        );
    }

    #[tokio::test]
    async fn the_books_of_a_series_load_what_the_scanner_wrote() {
        let temp = TempDatabase::open();
        let (series, book) = temp
            .db
            .setup_new_transaction(|conn| {
                conn.execute("INSERT INTO series (name) VALUES ('the long road')", [])
                    .unwrap();
                let series = conn.last_insert_rowid();
                // Like the scanner: no tags, no language, and a file time it couldn't read.
                conn.execute(
                    "INSERT INTO books (title, author, format, language, tags, file_path, \
                     volume_number, series) \
                     VALUES ('The Long Road', 'Unknown Author', 'cbz', 'Unknown', '', \
                     '/books/The Long Road v01.cbz', 1, ?1)",
                    [series],
                )
                .unwrap();
                Ok((series, conn.last_insert_rowid()))
            })
            .unwrap();
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        let uri = format!("/api/v1/series/{series}/books");
        let (status, _, body) = call(&app, authorized("GET", &uri, &member, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json(&body),
            serde_json::json!([{
                "id": book,
                "title": "The Long Road",
                "author": "Unknown Author",
                "format": "cbz",
                "tags": [],
                "description": null,
                "language": null,
                "page_count": null,
                "series": series,
                "volume_number": 1,
                "chapter_number": null,
                "page_number": null,
                "last_modified": null,
                "file_size": null,
            }])
        );
    }

    #[tokio::test]
    async fn a_books_answer_carries_no_server_path_or_hash() {
        let temp = TempDatabase::open();
        let book = temp
            .db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO books (title, author, format, language, cover_image, \
                     description, tags, file_path, page_count, volume_number, chapter_number, \
                     file_hash, last_modified, file_size) \
                     VALUES ('A Quiet Sea', 'R. Writer', 'epub', 'de', \
                     './cover_images/abc123.jpg', 'Twelve chapters', 'sea,quiet', \
                     '/Users/someone/books/A Quiet Sea v03 c12.epub', 240, 3, 12, 'abc123', \
                     1700000000, 1048576)",
                    [],
                )
                .unwrap();
                Ok(conn.last_insert_rowid())
            })
            .unwrap();
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        let uri = format!("/api/v1/books/{book}");
        let (status, _, body) = call(&app, authorized("GET", &uri, &member, None)).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json(&body),
            serde_json::json!({
                "title": "A Quiet Sea",
                "author": "R. Writer",
                "format": "epub",
                "tags": ["sea", "quiet"],
                "description": "Twelve chapters",
                "language": "de",
                "page_count": 240,
                "series": null,
                "volume_number": 3,
                "chapter_number": 12,
                "page_number": null,
                "last_modified": 1700000000,
                "file_size": 1048576,
                "reading_mode": "ltr",
            })
        );
    }

    #[tokio::test]
    async fn a_book_opens_in_the_callers_mode_for_its_series_over_http() {
        let temp = TempDatabase::open();
        let (series, manga, scan) = temp
            .db
            .setup_new_transaction(|conn| {
                conn.execute("INSERT INTO series (name) VALUES ('berserk')", [])
                    .unwrap();
                let series = conn.last_insert_rowid();
                let book = |format: &str, path: &str, direction: Option<&str>| {
                    conn.execute(
                        "INSERT INTO books (title, author, format, language, tags, file_path, \
                         series, reading_direction) \
                         VALUES ('Berserk', 'Kentaro Miura', ?1, 'ja', '', ?2, ?3, ?4)",
                        rusqlite::params![format, path, series, direction],
                    )
                    .unwrap();
                    conn.last_insert_rowid()
                };
                let manga = book("cbz", "/manga/Berserk v01.cbz", Some("rtl"));
                let scan = book("pdf", "/manga/Berserk v02.pdf", None);
                Ok((series, manga, scan))
            })
            .unwrap();
        let alice = logged_in(&temp, "alice", "member");
        let bob = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let mode_of = |book: i64, token: &str| {
            authorized("GET", &format!("/api/v1/books/{book}"), token, None)
        };
        let choose = |series: i64, body: serde_json::Value| {
            authorized(
                "PUT",
                &format!("/api/v1/series/{series}/reading_mode"),
                &alice,
                Some(body),
            )
        };
        async fn mode(app: &Router, request: Request<Body>) -> serde_json::Value {
            let (status, _, body) = call(app, request).await;
            assert_eq!(status, StatusCode::OK);
            json(&body)["reading_mode"].clone()
        }

        assert_eq!(
            mode(&app, mode_of(manga, &alice)).await,
            "rtl",
            "the file's"
        );
        assert_eq!(
            mode(&app, mode_of(scan, &alice)).await,
            "continuous",
            "a PDF's"
        );

        let (status, _, body) =
            call(&app, choose(series, serde_json::json!({ "mode": "ltr" }))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&body), serde_json::json!({ "mode": "ltr" }));
        assert_eq!(mode(&app, mode_of(manga, &alice)).await, "ltr");
        assert_eq!(
            mode(&app, mode_of(scan, &alice)).await,
            "ltr",
            "the whole series"
        );
        assert_eq!(mode(&app, mode_of(manga, &bob)).await, "rtl", "bob's own");

        for (body, key) in [
            (
                serde_json::json!({ "mode": "sideways" }),
                "invalid_reading_mode",
            ),
            (serde_json::json!({ "mode": "RTL" }), "invalid_reading_mode"),
            (serde_json::json!({ "mode": "" }), "invalid_reading_mode"),
            (serde_json::json!({ "mode": 3 }), "invalid_request"),
            (serde_json::json!({}), "invalid_request"),
        ] {
            let (status, _, answer) = call(&app, choose(series, body.clone())).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(json(&answer), serde_json::json!({ "error": key }), "{body}");
        }
        let (status, _, answer) = call(
            &app,
            choose(series + 1, serde_json::json!({ "mode": "rtl" })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json(&answer), serde_json::json!({ "error": "not_found" }));
        assert_eq!(mode(&app, mode_of(manga, &alice)).await, "ltr", "unchanged");
    }

    #[tokio::test]
    async fn the_series_and_book_lists_answer_in_reading_order() {
        let temp = TempDatabase::open();
        // Inserted against the expected order, so the id order would be the wrong one.
        let (library, alpha) = temp
            .db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO library (library_name, library_type) VALUES ('/books', 'books')",
                    [],
                )
                .unwrap();
                let library = conn.last_insert_rowid();
                let mut series = Vec::new();
                for name in ["zeta", "alpha"] {
                    conn.execute("INSERT INTO series (name) VALUES (?1)", [name])
                        .unwrap();
                    let id = conn.last_insert_rowid();
                    conn.execute(
                        "INSERT INTO library_elements (library_id, series_id) VALUES (?1, ?2)",
                        [library, id],
                    )
                    .unwrap();
                    series.push(id);
                }
                let alpha = series[1];
                for volume in [2, 1] {
                    conn.execute(
                        "INSERT INTO books (title, author, format, language, tags, file_path, \
                         volume_number, series) \
                         VALUES ('Alpha', 'Unknown Author', 'cbz', 'Unknown', '', ?1, ?2, ?3)",
                        rusqlite::params![format!("/books/Alpha v{volume}.cbz"), volume, alpha],
                    )
                    .unwrap();
                }
                Ok((library, alpha))
            })
            .unwrap();
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let field = |body: &[u8], name: &str| -> Vec<serde_json::Value> {
            json(body)
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row[name].clone())
                .collect()
        };

        let uri = format!("/api/v1/libraries/{library}/series");
        let (status, _, body) = call(&app, authorized("GET", &uri, &member, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(field(&body, "name"), ["alpha", "zeta"]);

        let uri = format!("/api/v1/series/{alpha}/books");
        let (status, _, body) = call(&app, authorized("GET", &uri, &member, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(field(&body, "volume_number"), [1, 2]);
    }

    #[tokio::test]
    async fn a_cover_comes_from_its_own_row_or_else_from_the_first_book_in_reading_order() {
        let temp = TempDatabase::open();
        // Each file holds its own name, so a body shows which cover was served. Rows name a file
        // in the cover folder.
        let cover = |name: &str| {
            temp.covers.write(name, name.as_bytes());
            name.to_string()
        };
        let [extra, two, three, zeta_own, zeta_book, library_own] = [
            "extra.png",
            "two.png",
            "three.png",
            "zeta-own.png",
            "zeta-book.png",
            "library-own.png",
        ]
        .map(cover);
        let (volume_three, beta, zeta, library, own_library) = temp
            .db
            .setup_new_transaction(move |conn| {
                let add = |sql: &str, params: &[&dyn rusqlite::ToSql]| {
                    conn.execute(sql, params).unwrap();
                    conn.last_insert_rowid()
                };
                let series = |name: &str, cover: Option<&str>| {
                    add(
                        "INSERT INTO series (name, cover_image) VALUES (?1, ?2)",
                        &[&name, &cover],
                    )
                };
                let book = |series: i64, volume: Option<i64>, cover: Option<&str>| {
                    add(
                        "INSERT INTO books (title, author, format, language, tags, file_path, \
                         volume_number, series, cover_image) \
                         VALUES ('Book', 'Unknown Author', 'cbz', 'Unknown', '', ?1, ?2, ?3, ?4)",
                        &[&uuid::Uuid::new_v4().to_string(), &volume, &series, &cover],
                    )
                };
                let library = |cover: Option<&str>| {
                    add(
                        "INSERT INTO library (library_name, library_type, cover_image) \
                         VALUES ('books', 'books', ?1)",
                        &[&cover],
                    )
                };
                let link = |library: i64, series: i64| {
                    add(
                        "INSERT INTO library_elements (library_id, series_id) VALUES (?1, ?2)",
                        &[&library, &series],
                    );
                };

                // Beta's lowest id has a cover but no volume, and its volume 1 has no cover.
                let beta = series("Beta", None);
                book(beta, None, Some(extra.as_str()));
                book(beta, Some(1), None);
                let volume_three = book(beta, Some(3), Some(three.as_str()));
                book(beta, Some(2), Some(two.as_str()));
                let zeta = series("zeta", Some(zeta_own.as_str()));
                book(zeta, Some(1), Some(zeta_book.as_str()));
                let alpha = series("alpha", None);

                // "alpha" comes first by name but has no cover at all.
                let library_id = library(None);
                for series in [zeta, beta, alpha] {
                    link(library_id, series);
                }
                let own_library = library(Some(library_own.as_str()));
                link(own_library, zeta);
                Ok((volume_three, beta, zeta, library_id, own_library))
            })
            .unwrap();
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        for (uri, expected) in [
            (format!("/api/v1/covers/book/{volume_three}"), "three.png"),
            (format!("/api/v1/covers/series/{beta}"), "two.png"),
            (format!("/api/v1/covers/series/{zeta}"), "zeta-own.png"),
            (format!("/api/v1/covers/library/{library}"), "two.png"),
            (
                format!("/api/v1/covers/library/{own_library}"),
                "library-own.png",
            ),
        ] {
            let (status, headers, body) = call(&app, authorized("GET", &uri, &member, None)).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert_eq!(headers["content-type"], "image/png", "{uri}");
            assert_eq!(body, expected.as_bytes(), "{uri}");
        }
    }

    #[tokio::test]
    async fn a_cover_that_is_not_there_answers_404_json_and_an_unknown_kind_400_json() {
        let temp = TempDatabase::open();
        // A name in the cover folder whose file isn't there.
        let gone = "gone.png".to_string();
        let (coverless_book, gone_book, empty_series, empty_library) = temp
            .db
            .setup_new_transaction(move |conn| {
                let add = |sql: &str, params: &[&dyn rusqlite::ToSql]| {
                    conn.execute(sql, params).unwrap();
                    conn.last_insert_rowid()
                };
                let book = |cover: Option<&str>| {
                    add(
                        "INSERT INTO books (title, author, format, language, tags, file_path, \
                         cover_image) \
                         VALUES ('Book', 'Unknown Author', 'cbz', 'Unknown', '', ?1, ?2)",
                        &[&uuid::Uuid::new_v4().to_string(), &cover],
                    )
                };
                let coverless_book = book(None);
                let gone_book = book(Some(gone.as_str()));
                let empty_series = add("INSERT INTO series (name) VALUES ('empty')", &[]);
                let empty_library = add(
                    "INSERT INTO library (library_name, library_type) VALUES ('empty', 'books')",
                    &[],
                );
                add(
                    "INSERT INTO library_elements (library_id, series_id) VALUES (?1, ?2)",
                    &[&empty_library, &empty_series],
                );
                Ok((coverless_book, gone_book, empty_series, empty_library))
            })
            .unwrap();
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        for uri in [
            format!("/api/v1/covers/book/{coverless_book}"),
            format!("/api/v1/covers/book/{gone_book}"),
            "/api/v1/covers/book/999".to_string(),
            format!("/api/v1/covers/series/{empty_series}"),
            "/api/v1/covers/series/999".to_string(),
            format!("/api/v1/covers/library/{empty_library}"),
            "/api/v1/covers/library/999".to_string(),
        ] {
            let (status, _, body) = call(&app, authorized("GET", &uri, &member, None)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(
                json(&body),
                serde_json::json!({ "error": "not_found" }),
                "{uri}"
            );
        }

        for uri in [
            "/api/v1/covers/author/1",
            "/api/v1/covers/Book/1",
            "/api/v1/covers/book/abc",
        ] {
            let (status, _, body) = call(&app, authorized("GET", uri, &member, None)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(
                json(&body),
                serde_json::json!({ "error": "invalid_request" }),
                "{uri}"
            );
        }
    }

    /// A series in a library, with two volumes covered by `first.png` and `second.png`; each file
    /// holds its own name. Answers the series', the second volume's and the library's ids.
    fn covered_series(temp: &TempDatabase) -> (i64, i64, i64) {
        for name in ["first.png", "second.png"] {
            temp.covers.write(name, name.as_bytes());
        }
        temp.db
            .setup_new_transaction(|conn| {
                conn.execute("INSERT INTO series (name) VALUES ('berserk')", [])
                    .unwrap();
                let series = conn.last_insert_rowid();
                for (volume, cover) in [(1, "first.png"), (2, "second.png")] {
                    conn.execute(
                        "INSERT INTO books (title, author, format, language, tags, file_path, \
                         volume_number, series, cover_image) \
                         VALUES ('Berserk', 'Kentaro Miura', 'cbz', 'ja', '', ?1, ?2, ?3, ?4)",
                        rusqlite::params![
                            format!("/books/Berserk v{volume}.cbz"),
                            volume,
                            series,
                            cover
                        ],
                    )
                    .unwrap();
                }
                let second = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO library (library_name, library_type) VALUES ('manga', 'comics')",
                    [],
                )
                .unwrap();
                let library = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO library_elements (library_id, series_id) VALUES (?1, ?2)",
                    [library, series],
                )
                .unwrap();
                Ok((series, second, library))
            })
            .unwrap()
    }

    /// The body of the cover that `GET uri` serves.
    async fn served_cover(app: &Router, token: &str, uri: &str) -> Vec<u8> {
        let (status, _, body) = call(app, authorized("GET", uri, token, None)).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        body
    }

    #[tokio::test]
    async fn an_admin_chooses_covers_and_the_cover_route_serves_them_until_they_are_deleted() {
        let temp = TempDatabase::open();
        let admin = logged_in(&temp, "alice", "admin");
        let (series, second, library) = covered_series(&temp);
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let series_cover = format!("/api/v1/series/{series}/cover");
        let library_cover = format!("/api/v1/libraries/{library}/cover");
        let shown_for_series = format!("/api/v1/covers/series/{series}");
        let shown_for_library = format!("/api/v1/covers/library/{library}");

        for (uri, body) in [
            (&series_cover, serde_json::json!({ "book_id": second })),
            (&library_cover, serde_json::json!({ "series_id": series })),
        ] {
            let (status, _, answer) = call(&app, authorized("PUT", uri, &admin, Some(body))).await;
            assert_eq!(status, StatusCode::NO_CONTENT, "{uri}");
            assert!(answer.is_empty(), "{uri}");
        }
        assert_eq!(
            served_cover(&app, &admin, &shown_for_series).await,
            b"second.png"
        );
        assert_eq!(
            served_cover(&app, &admin, &shown_for_library).await,
            b"second.png"
        );

        let (status, _, _) = call(&app, authorized("DELETE", &series_cover, &admin, None)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(
            served_cover(&app, &admin, &shown_for_series).await,
            b"first.png",
            "the first volume's again"
        );
        assert_eq!(
            served_cover(&app, &admin, &shown_for_library).await,
            b"second.png",
            "the library keeps the picture it was given"
        );

        let (status, _, _) = call(&app, authorized("DELETE", &library_cover, &admin, None)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(
            served_cover(&app, &admin, &shown_for_library).await,
            b"first.png",
            "its series' again"
        );
    }

    #[tokio::test]
    async fn a_cover_choice_answers_400_or_404_json_and_only_an_admin_makes_one() {
        let temp = TempDatabase::open();
        let admin = logged_in(&temp, "alice", "admin");
        let member = logged_in(&temp, "bob", "member");
        let (series, second, library) = covered_series(&temp);
        let monster = scanned_series(&temp, "monster");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let series_cover = format!("/api/v1/series/{series}/cover");
        let library_cover = format!("/api/v1/libraries/{library}/cover");

        for (method, uri, body, status, key) in [
            // A book no series has, a body without `book_id`, a series the library doesn't hold.
            (
                "PUT",
                series_cover.clone(),
                Some(serde_json::json!({ "book_id": second + 100 })),
                StatusCode::BAD_REQUEST,
                "invalid_request",
            ),
            (
                "PUT",
                series_cover.clone(),
                Some(serde_json::json!({ "series_id": series })),
                StatusCode::BAD_REQUEST,
                "invalid_request",
            ),
            (
                "PUT",
                library_cover.clone(),
                Some(serde_json::json!({ "series_id": monster })),
                StatusCode::BAD_REQUEST,
                "invalid_request",
            ),
            (
                "PUT",
                "/api/v1/series/999/cover".to_string(),
                Some(serde_json::json!({ "book_id": second })),
                StatusCode::NOT_FOUND,
                "not_found",
            ),
            (
                "DELETE",
                "/api/v1/series/999/cover".to_string(),
                None,
                StatusCode::NOT_FOUND,
                "not_found",
            ),
            (
                "PUT",
                "/api/v1/libraries/999/cover".to_string(),
                Some(serde_json::json!({ "series_id": series })),
                StatusCode::NOT_FOUND,
                "not_found",
            ),
            (
                "DELETE",
                "/api/v1/libraries/999/cover".to_string(),
                None,
                StatusCode::NOT_FOUND,
                "not_found",
            ),
        ] {
            let (answered, _, answer) = call(&app, authorized(method, &uri, &admin, body)).await;
            assert_eq!(answered, status, "{method} {uri}");
            assert_eq!(
                json(&answer),
                serde_json::json!({ "error": key }),
                "{method} {uri}"
            );
        }

        for (method, uri, body) in [
            (
                "PUT",
                &series_cover,
                Some(serde_json::json!({ "book_id": second })),
            ),
            ("DELETE", &series_cover, None),
            (
                "PUT",
                &library_cover,
                Some(serde_json::json!({ "series_id": series })),
            ),
            ("DELETE", &library_cover, None),
        ] {
            let (status, _, answer) = call(&app, authorized(method, uri, &member, body)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
            assert_eq!(json(&answer), serde_json::json!({ "error": "forbidden" }));
        }

        let stored: (Option<String>, Option<String>) = temp
            .db
            .setup_new_transaction(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT (SELECT cover_image FROM series WHERE id = ?1), \
                         (SELECT cover_image FROM library WHERE id = ?2)",
                        [series, library],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap())
            })
            .unwrap();
        assert_eq!(stored, (None, None), "nothing refused was stored");
    }

    #[tokio::test]
    async fn a_books_file_streams_and_a_folder_or_a_file_gone_answers_404_json() {
        let temp = TempDatabase::open();
        let files = TempDir::new();
        let path = |relative: &str| files.path().join(relative).to_string_lossy().into_owned();
        files.write("A Quiet Sea.pdf", b"%PDF-1.7 tiny");
        files.write("The Long Road/001.png", b"page");
        let (pdf, folder, gone) = (
            path("A Quiet Sea.pdf"),
            path("The Long Road"),
            path("Gone.pdf"),
        );
        let books = files.path().to_path_buf();
        let (pdf_book, folder_book, gone_book) = temp
            .db
            .setup_new_transaction(move |conn| {
                let directory = insert_directory_row(conn, &books);
                Ok((
                    insert_book_file(conn, directory, "pdf", &pdf),
                    insert_book_file(conn, directory, "image_comic", &folder),
                    insert_book_file(conn, directory, "pdf", &gone),
                ))
            })
            .unwrap();
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        let uri = format!("/api/v1/books/{pdf_book}/file");
        let (status, headers, body) = call(&app, authorized("GET", &uri, &member, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["content-type"], "application/pdf");
        assert_eq!(body, b"%PDF-1.7 tiny");

        // An image-folder book's stored path is its folder.
        for book in [folder_book, gone_book] {
            let uri = format!("/api/v1/books/{book}/file");
            let (status, _, body) = call(&app, authorized("GET", &uri, &member, None)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(
                json(&body),
                serde_json::json!({ "error": "not_found" }),
                "{uri}"
            );
        }
    }

    /// A directory row for `folder`, written with plain SQL; answers its id.
    fn insert_directory_row(conn: &rusqlite::Connection, folder: &std::path::Path) -> i64 {
        conn.execute(
            "INSERT INTO directories (path) VALUES (?1)",
            [folder.to_string_lossy()],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    /// A book of `format` at `file_path` in `directory`, written with plain SQL; answers its id.
    fn insert_book_file(
        conn: &rusqlite::Connection,
        directory: impl Into<Option<i64>>,
        format: &str,
        file_path: &str,
    ) -> i64 {
        conn.execute(
            "INSERT INTO books (title, author, format, language, tags, file_path, directory_id) \
             VALUES ('Book', 'Unknown Author', ?1, 'Unknown', '', ?2, ?3)",
            rusqlite::params![format, file_path, directory.into()],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[tokio::test]
    async fn the_pages_of_an_image_folder_and_a_cbz_are_listed_and_served_in_natural_order() {
        let temp = TempDatabase::open();
        let files = TempDir::new();
        for (name, contents) in [("10.png", "ten"), ("2.png", "two"), ("1.png", "one")] {
            files.write(&format!("The Long Road/{name}"), contents.as_bytes());
        }
        let cbz = files.write_cbz(
            "A Quiet Sea.cbz",
            &[("p10.webp", b"ten"), ("p2.jpg", b"two")],
        );
        let folder = files.path().join("The Long Road");
        let (folder, cbz) = (
            folder.to_string_lossy().into_owned(),
            cbz.to_string_lossy().into_owned(),
        );
        let books = files.path().to_path_buf();
        let (folder_book, cbz_book) = temp
            .db
            .setup_new_transaction(move |conn| {
                let directory = insert_directory_row(conn, &books);
                Ok((
                    insert_book_file(conn, directory, "image_comic", &folder),
                    insert_book_file(conn, directory, "cbz", &cbz),
                ))
            })
            .unwrap();
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        for (book, count) in [(folder_book, 3), (cbz_book, 2)] {
            let uri = format!("/api/v1/books/{book}/pages");
            let (status, _, body) = call(&app, authorized("GET", &uri, &member, None)).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            // The pages hold words, not images, so none has a size.
            let numbers: Vec<serde_json::Value> = (1..=count)
                .map(
                    |number| serde_json::json!({ "number": number, "width": null, "height": null }),
                )
                .collect();
            assert_eq!(json(&body), serde_json::Value::Array(numbers), "{uri}");
        }

        for (book, number, content_type, contents) in [
            (folder_book, 1, "image/png", "one"),
            (folder_book, 3, "image/png", "ten"),
            (cbz_book, 1, "image/jpeg", "two"),
            (cbz_book, 2, "image/webp", "ten"),
        ] {
            let uri = format!("/api/v1/books/{book}/pages/{number}");
            let (status, headers, body) = call(&app, authorized("GET", &uri, &member, None)).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert_eq!(headers["content-type"], content_type, "{uri}");
            assert_eq!(body, contents.as_bytes(), "{uri}");
        }
    }

    #[tokio::test]
    async fn a_books_page_list_carries_each_pages_size_and_null_where_none_can_be_read() {
        let temp = TempDatabase::open();
        let files = TempDir::new();
        let jpeg = encoded_image(image::ImageFormat::Jpeg, 120, 180);
        let png = encoded_image(image::ImageFormat::Png, 180, 120);
        files.write("The Long Road/1.jpg", &jpeg);
        files.write("The Long Road/2.png", &png);
        let cbz = files.write_cbz(
            "A Quiet Sea.cbz",
            &[("1.png", &png), ("2.jpg", b"broken"), ("3.jpg", &jpeg)],
        );
        let folder = files.path().join("The Long Road");
        let (folder, cbz) = (
            folder.to_string_lossy().into_owned(),
            cbz.to_string_lossy().into_owned(),
        );
        let books = files.path().to_path_buf();
        let (folder_book, cbz_book) = temp
            .db
            .setup_new_transaction(move |conn| {
                let directory = insert_directory_row(conn, &books);
                Ok((
                    insert_book_file(conn, directory, "image_comic", &folder),
                    insert_book_file(conn, directory, "cbz", &cbz),
                ))
            })
            .unwrap();
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        for (book, expected) in [
            (
                folder_book,
                serde_json::json!([
                    { "number": 1, "width": 120, "height": 180 },
                    { "number": 2, "width": 180, "height": 120 },
                ]),
            ),
            (
                cbz_book,
                serde_json::json!([
                    { "number": 1, "width": 180, "height": 120 },
                    { "number": 2, "width": null, "height": null },
                    { "number": 3, "width": 120, "height": 180 },
                ]),
            ),
        ] {
            let uri = format!("/api/v1/books/{book}/pages");
            let (status, _, body) = call(&app, authorized("GET", &uri, &member, None)).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert_eq!(json(&body), expected, "{uri}");
        }
    }

    #[tokio::test]
    async fn a_page_that_is_not_there_answers_404_json_and_a_broken_archive_500_json() {
        let temp = TempDatabase::open();
        let files = TempDir::new();
        let path = |relative: &str| files.path().join(relative).to_string_lossy().into_owned();
        files.write("The Long Road/1.png", b"one");
        files.write("A Quiet Sea.pdf", b"%PDF-1.7");
        files.write("Broken.cbz", b"Rar! renamed to cbz");
        let (folder, pdf, broken, gone) = (
            path("The Long Road"),
            path("A Quiet Sea.pdf"),
            path("Broken.cbz"),
            path("Gone"),
        );
        let books = files.path().to_path_buf();
        let (folder_book, pdf_book, broken_book, gone_book) = temp
            .db
            .setup_new_transaction(move |conn| {
                let directory = insert_directory_row(conn, &books);
                Ok((
                    insert_book_file(conn, directory, "image_comic", &folder),
                    insert_book_file(conn, directory, "pdf", &pdf),
                    insert_book_file(conn, directory, "cbz", &broken),
                    insert_book_file(conn, directory, "image_comic", &gone),
                ))
            })
            .unwrap();
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let answer = |status: StatusCode, key: &str| (status, serde_json::json!({ "error": key }));

        let cases = [
            (
                format!("/api/v1/books/{folder_book}/pages/0"),
                answer(StatusCode::NOT_FOUND, "not_found"),
            ),
            (
                format!("/api/v1/books/{folder_book}/pages/2"),
                answer(StatusCode::NOT_FOUND, "not_found"),
            ),
            (
                format!("/api/v1/books/{pdf_book}/pages"),
                answer(StatusCode::NOT_FOUND, "not_found"),
            ),
            (
                format!("/api/v1/books/{pdf_book}/pages/1"),
                answer(StatusCode::NOT_FOUND, "not_found"),
            ),
            (
                format!("/api/v1/books/{gone_book}/pages"),
                answer(StatusCode::NOT_FOUND, "not_found"),
            ),
            (
                "/api/v1/books/999/pages".to_string(),
                answer(StatusCode::NOT_FOUND, "not_found"),
            ),
            (
                "/api/v1/books/999/pages/1".to_string(),
                answer(StatusCode::NOT_FOUND, "not_found"),
            ),
            (
                format!("/api/v1/books/{folder_book}/pages/abc"),
                answer(StatusCode::BAD_REQUEST, "invalid_request"),
            ),
            (
                format!("/api/v1/books/{folder_book}/pages/-1"),
                answer(StatusCode::BAD_REQUEST, "invalid_request"),
            ),
            (
                format!("/api/v1/books/{broken_book}/pages"),
                answer(StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
            ),
            (
                format!("/api/v1/books/{broken_book}/pages/1"),
                answer(StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
            ),
        ];
        for (uri, (expected_status, expected_body)) in cases {
            let (status, _, body) = call(&app, authorized("GET", &uri, &member, None)).await;
            assert_eq!(status, expected_status, "{uri}");
            assert_eq!(json(&body), expected_body, "{uri}");
        }
    }

    #[tokio::test]
    async fn a_file_outside_where_the_server_serves_from_answers_404_json_on_every_route() {
        let temp = TempDatabase::open();
        let files = TempDir::new();
        let path = |relative: &str| files.path().join(relative).to_string_lossy().into_owned();
        files.write_cbz("books/Unplaced.cbz", &[("1.png", b"page")]);
        files.write_cbz("elsewhere/Secret.cbz", &[("1.png", b"secret page")]);
        files.write("elsewhere/secret.png", b"secret cover");
        // A cover name that climbs out of the cover folder into `files`.
        let climbing = format!(
            "../{}/elsewhere/secret.png",
            files.path().file_name().unwrap().to_string_lossy()
        );
        let (books, unplaced, outside) = (
            files.path().join("books"),
            path("books/Unplaced.cbz"),
            path("elsewhere/Secret.cbz"),
        );
        let (unplaced_book, outside_book) = temp
            .db
            .setup_new_transaction(move |conn| {
                let directory = insert_directory_row(conn, &books);
                let outside_book = insert_book_file(conn, directory, "cbz", &outside);
                conn.execute(
                    "UPDATE books SET cover_image = ?1 WHERE id = ?2",
                    rusqlite::params![climbing, outside_book],
                )
                .unwrap();
                Ok((insert_book_file(conn, None, "cbz", &unplaced), outside_book))
            })
            .unwrap();
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        let mut uris = vec![format!("/api/v1/covers/book/{outside_book}")];
        for book in [unplaced_book, outside_book] {
            for route in ["file", "pages", "pages/1"] {
                uris.push(format!("/api/v1/books/{book}/{route}"));
            }
        }
        for uri in uris {
            let (status, _, body) = call(&app, authorized("GET", &uri, &member, None)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(
                json(&body),
                serde_json::json!({ "error": "not_found" }),
                "{uri}"
            );
        }
    }

    #[tokio::test]
    async fn a_body_that_cannot_be_read_answers_400_json() {
        let temp = TempDatabase::open();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let login = |content_type: Option<&str>, body: &str| {
            let mut request = Request::post("/api/v1/auth/login");
            if let Some(content_type) = content_type {
                request = request.header("content-type", content_type);
            }
            request.body(Body::from(body.to_string())).unwrap()
        };
        let json_type = Some("application/json");

        for (label, request) in [
            (
                "malformed JSON",
                login(json_type, r#"{"username_or_email":"#),
            ),
            ("an empty body", login(json_type, "")),
            (
                "a missing field",
                login(json_type, r#"{"username_or_email":"alice"}"#),
            ),
            (
                "a wrong field type",
                login(
                    json_type,
                    r#"{"username_or_email":"alice","password":123456,"device_name":"phone","platform":"ios"}"#,
                ),
            ),
            (
                "no content type",
                login(
                    None,
                    r#"{"username_or_email":"alice","password":"correct horse battery","device_name":"phone","platform":"ios"}"#,
                ),
            ),
        ] {
            let (status, _, body) = call(&app, request).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{label}");
            // Exactly the key: the parse error, which can quote the sent value, stays out.
            assert_eq!(
                json(&body),
                serde_json::json!({ "error": "invalid_request" }),
                "{label}"
            );
        }
    }

    #[tokio::test]
    async fn a_guarded_route_reads_its_body_only_after_the_session() {
        let temp = TempDatabase::open();
        let user = temp
            .db
            .setup_new_transaction(|conn| Ok(insert_user(conn, "alice")))
            .unwrap();
        add_session(
            &temp,
            user,
            "d1",
            "session-1",
            "refresh-1",
            unix_now() + 600,
        );
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let broken_body = |token: Option<&str>| {
            let mut request = Request::post("/api/v1/account/totp/confirm")
                .header("content-type", "application/json");
            if let Some(token) = token {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            request.body(Body::from(r#"{"code":"#)).unwrap()
        };

        let (status, _, _) = call(&app, broken_body(None)).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "no session, no body reading"
        );

        let (status, _, body) = call(&app, broken_body(Some("session-1"))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json(&body),
            serde_json::json!({ "error": "invalid_request" })
        );
    }

    #[tokio::test]
    async fn an_id_that_is_not_a_number_answers_400_json() {
        let temp = TempDatabase::open();
        let admin = logged_in(&temp, "alice", "admin");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        for (method, uri) in [
            ("GET", "/api/v1/books/abc"),
            ("GET", "/api/v1/books/abc/file"),
            ("GET", "/api/v1/libraries/abc/series"),
            ("GET", "/api/v1/series/abc/books"),
            ("DELETE", "/api/v1/libraries/abc"),
            ("GET", "/api/v1/books/99999999999999999999"),
        ] {
            let (status, _, body) = call(&app, authorized(method, uri, &admin, None)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{method} {uri}");
            assert_eq!(
                json(&body),
                serde_json::json!({ "error": "invalid_request" }),
                "{method} {uri}"
            );
        }
    }

    fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::post(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn a_reset_request_without_email_configured_answers_503() {
        let (status, body) = json_answer(post_json(
            "/api/v1/auth/password_reset",
            serde_json::json!({ "email": "alice@example.com" }),
        ))
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body, serde_json::json!({ "error": "email_not_configured" }));
    }

    #[tokio::test]
    async fn known_and_unknown_addresses_get_the_same_answer_and_only_one_mail_goes_out() {
        let temp = TempDatabase::open();
        temp.db
            .setup_new_transaction(|conn| Ok(insert_user(conn, "alice")))
            .unwrap();
        let (mailer, sent) = Mailer::capture();
        let app = set_up_routes().with_state(temp.state(mailer));

        let mut answers = Vec::new();
        for email in ["alice@example.com", "nobody@example.com"] {
            let response = app
                .clone()
                .oneshot(post_json(
                    "/api/v1/auth/password_reset",
                    serde_json::json!({ "email": email }),
                ))
                .await
                .unwrap();
            let status = response.status();
            let body = to_bytes(response.into_body(), 1024).await.unwrap();
            answers.push((status, body));
        }
        assert_eq!(answers[0].0, StatusCode::ACCEPTED);
        assert_eq!(answers[0], answers[1], "the two answers must be identical");

        // The mail goes out in a background task; give it up to two seconds.
        for _ in 0..200 {
            if !sent.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].email, "alice@example.com");
    }

    #[tokio::test]
    async fn too_many_logins_answer_429_with_retry_after() {
        let temp = TempDatabase::open();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let wrong_login = || {
            post_json(
                "/api/v1/auth/login",
                serde_json::json!({
                    "username_or_email": "nobody",
                    "password": "wrong password 123",
                    "device_name": "phone",
                    "platform": "ios",
                }),
            )
        };
        for _ in 0..10 {
            let (status, _, _) = call(&app, wrong_login()).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }

        let (status, headers, body) = call(&app, wrong_login()).await;

        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            json(&body),
            serde_json::json!({ "error": "too_many_attempts" })
        );
        let seconds: u64 = headers["retry-after"].to_str().unwrap().parse().unwrap();
        assert!(
            (890..=900).contains(&seconds),
            "about the 15-minute window: {seconds}"
        );
    }

    #[tokio::test]
    async fn a_reset_with_a_bad_code_answers_400_json() {
        let (status, body) = json_answer(post_json(
            "/api/v1/auth/password_reset/confirm",
            serde_json::json!({
                "email": "nobody@example.com",
                "code": "ABCD2345",
                "new_password": "new password 456",
                "new_password_repeat": "new password 456",
            }),
        ))
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            serde_json::json!({ "error": "invalid_or_expired_code" })
        );
    }

    fn unix_now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    fn add_session(
        temp: &TempDatabase,
        user_id: i64,
        device: &str,
        session_token: &str,
        refresh_token: &str,
        session_expires_at: i64,
    ) {
        let device = device.to_string();
        let session_hash = hash_token(session_token);
        let refresh_hash = hash_token(refresh_token);
        temp.db
            .setup_new_transaction(move |conn| {
                conn.execute(
                    "INSERT INTO sessions (user_id, device_id, device_name, platform, \
                     refresh_token_hashed, refresh_token_valid_until, session_token_hashed, \
                     session_token_valid_until, last_used_at) \
                     VALUES (?1, ?2, 'phone', 'ios', ?3, ?4, ?5, ?6, 0)",
                    rusqlite::params![
                        user_id,
                        device,
                        refresh_hash,
                        session_expires_at + 3600,
                        session_hash,
                        session_expires_at
                    ],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
    }

    async fn call(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), 4096).await.unwrap().to_vec();
        (status, headers, body)
    }

    /// A request to a route behind `AuthSession` that answers 200 for any live session.
    fn guarded(authorization: Option<&str>) -> Request<Body> {
        let mut request = Request::get("/api/v1/account/totp");
        if let Some(value) = authorization {
            request = request.header("authorization", value);
        }
        request.body(Body::empty()).unwrap()
    }

    fn post_with_bearer(uri: &str, token: &str) -> Request<Body> {
        Request::post(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn logging_in_over_http_returns_tokens_that_open_guarded_routes() {
        let temp = TempDatabase::open();
        let hash = hash_secret("correct horse battery".to_string())
            .await
            .unwrap();
        temp.db
            .setup_new_transaction(move |conn| {
                conn.execute(
                    "INSERT INTO users (username, password_hash, email, email_verified) \
                     VALUES ('alice', ?1, 'alice@example.com', 1)",
                    [hash],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        let (status, headers, body) = call(
            &app,
            post_json(
                "/api/v1/auth/login",
                serde_json::json!({
                    "username_or_email": "alice",
                    "password": "correct horse battery",
                    "device_name": "phone",
                    "platform": "ios",
                }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get("cache-control").unwrap(), "no-store");
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["status"], "ok");
        assert!(body["session_expires_at"].is_i64());
        assert!(body["refresh_token"].is_string());
        assert!(body["device_id"].is_string());

        let session_token = body["session_token"].as_str().unwrap();
        let (status, _, _) = call(&app, guarded(Some(&format!("Bearer {session_token}")))).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_guarded_route_accepts_only_a_live_bearer_session_token() {
        let temp = TempDatabase::open();
        let user = temp
            .db
            .setup_new_transaction(|conn| Ok(insert_user(conn, "alice")))
            .unwrap();
        add_session(
            &temp,
            user,
            "d1",
            "live-token",
            "live-refresh",
            unix_now() + 600,
        );
        add_session(
            &temp,
            user,
            "d2",
            "old-token",
            "old-refresh",
            unix_now() - 1,
        );
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        for accepted in ["Bearer live-token", "bearer live-token"] {
            let (status, _, _) = call(&app, guarded(Some(accepted))).await;
            assert_eq!(status, StatusCode::OK, "{accepted}");
        }

        let mut refusals = Vec::new();
        for refused in [
            None,
            Some("Basic live-token"),
            Some("Bearer "),
            Some("Bearer unknown"),
            Some("Bearer old-token"),
            Some("Bearer live-refresh"),
        ] {
            let (status, _, body) = call(&app, guarded(refused)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{refused:?}");
            refusals.push(body);
        }
        assert!(
            refusals.windows(2).all(|pair| pair[0] == pair[1]),
            "every refusal must look the same"
        );
    }

    #[tokio::test]
    async fn refresh_and_logout_over_http() {
        let temp = TempDatabase::open();
        let user = temp
            .db
            .setup_new_transaction(|conn| Ok(insert_user(conn, "alice")))
            .unwrap();
        add_session(
            &temp,
            user,
            "d1",
            "session-1",
            "refresh-1",
            unix_now() + 600,
        );
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        let (status, _, _) =
            call(&app, post_with_bearer("/api/v1/auth/refresh", "session-1")).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "a session token is not a refresh token"
        );

        let (status, headers, body) =
            call(&app, post_with_bearer("/api/v1/auth/refresh", "refresh-1")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get("cache-control").unwrap(), "no-store");
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let new_session = body["session_token"].as_str().unwrap().to_string();
        let new_refresh = body["refresh_token"].as_str().unwrap().to_string();
        assert_eq!(body["device_id"], "d1");

        let (status, _, _) = call(&app, guarded(Some(&format!("Bearer {new_session}")))).await;
        assert_eq!(status, StatusCode::OK);

        let (status, _, _) =
            call(&app, post_with_bearer("/api/v1/auth/logout", &new_refresh)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _, _) = call(&app, guarded(Some(&format!("Bearer {new_session}")))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "logged out");

        let without_token = Request::post("/api/v1/auth/logout")
            .body(Body::empty())
            .unwrap();
        let (status, _, _) = call(&app, without_token).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    /// A sign-up body that carries the setup code; only the first account needs it.
    fn sign_up_body(username: &str, email: &str) -> serde_json::Value {
        serde_json::json!({
            "email": email,
            "username": username,
            "password": "correct horse battery",
            "password_repeat": "correct horse battery",
            "device_name": "phone",
            "platform": "ios",
            "setup_code": TEST_SETUP_CODE,
        })
    }

    fn json(body: &[u8]) -> serde_json::Value {
        serde_json::from_slice(body).unwrap()
    }

    #[tokio::test]
    async fn a_first_sign_up_without_the_setup_code_answers_403_json() {
        let mut body = sign_up_body("alice", "alice@example.com");
        body.as_object_mut().unwrap().remove("setup_code");

        let (status, body) = json_answer(post_json("/api/v1/auth/register", body)).await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body, serde_json::json!({ "error": "invalid_setup_code" }));
    }

    #[tokio::test]
    async fn a_sign_up_over_http_stores_its_language_and_refuses_one_not_offered() {
        let temp = TempDatabase::open();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let mut body = sign_up_body("alice", "alice@example.com");

        body["locale"] = serde_json::json!("fr");
        let (status, _, answer) =
            call(&app, post_json("/api/v1/auth/register", body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json(&answer),
            serde_json::json!({ "error": "invalid_locale" })
        );

        body["locale"] = serde_json::json!("de");
        let (status, _, answer) = call(&app, post_json("/api/v1/auth/register", body)).await;
        assert_eq!(status, StatusCode::OK);
        let token = json(&answer)["session_token"].as_str().unwrap().to_string();
        let (status, _, settings) = call(
            &app,
            authorized("GET", "/api/v1/account/settings", &token, None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&settings), serde_json::json!({ "locale": "de" }));
    }

    #[tokio::test]
    async fn signing_up_and_verifying_over_http() {
        let temp = TempDatabase::open();
        let (mailer, sent) = Mailer::capture();
        let app = set_up_routes().with_state(temp.state(mailer));
        let register = |username: &str| {
            post_json(
                "/api/v1/auth/register",
                sign_up_body(username, &format!("{username}@example.com")),
            )
        };

        let (status, headers, body) = call(&app, register("alice")).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the first account is verified on the spot"
        );
        assert_eq!(headers.get("cache-control").unwrap(), "no-store");
        assert_eq!(json(&body)["status"], "ok");

        let (status, _, body) = call(&app, register("bob")).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(
            json(&body),
            serde_json::json!({ "verification_required": true })
        );
        let code = code_in(&sent.lock().unwrap()[0]);

        let (status, headers, body) = call(
            &app,
            post_json(
                "/api/v1/auth/verify_email",
                serde_json::json!({
                    "username_or_email": "bob",
                    "password": "correct horse battery",
                    "code": code,
                    "device_name": "phone",
                    "platform": "ios",
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get("cache-control").unwrap(), "no-store");
        let session_token = json(&body)["session_token"].as_str().unwrap().to_string();
        let (status, _, _) = call(&app, guarded(Some(&format!("Bearer {session_token}")))).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_role_or_verified_flag_in_a_sign_up_body_is_ignored() {
        let temp = TempDatabase::open();
        temp.db
            .setup_new_transaction(|conn| Ok(insert_user(conn, "alice")))
            .unwrap();
        let (mailer, _sent) = Mailer::capture();
        let app = set_up_routes().with_state(temp.state(mailer));
        let mut body = sign_up_body("bob", "bob@example.com");
        body["role"] = "admin".into();
        body["email_verified"] = true.into();

        let (status, _, _) = call(&app, post_json("/api/v1/auth/register", body)).await;

        assert_eq!(status, StatusCode::ACCEPTED);
        let stored: (String, bool) = temp
            .db
            .setup_new_transaction(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT role, email_verified FROM users WHERE username = 'bob'",
                        [],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap())
            })
            .unwrap();
        assert_eq!(stored, ("member".to_string(), false));
    }

    #[tokio::test]
    async fn sign_up_errors_answer_json() {
        let temp = TempDatabase::open();
        temp.db
            .setup_new_transaction(|conn| Ok(insert_user(conn, "alice")))
            .unwrap();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        let cases = [
            (
                sign_up_body("bob", "ALICE@example.com"),
                StatusCode::CONFLICT,
                "email_taken",
            ),
            (
                sign_up_body("Alice", "bob@example.com"),
                StatusCode::CONFLICT,
                "username_taken",
            ),
            (
                sign_up_body("bob", "bob@example.com\r\nDATA"),
                StatusCode::BAD_REQUEST,
                "invalid_email",
            ),
            (
                sign_up_body("bo", "bob@example.com"),
                StatusCode::BAD_REQUEST,
                "invalid_username",
            ),
        ];
        for (body, expected_status, expected_key) in cases {
            let (status, _, body) = call(&app, post_json("/api/v1/auth/register", body)).await;
            assert_eq!(status, expected_status, "{expected_key}");
            assert_eq!(json(&body), serde_json::json!({ "error": expected_key }));
        }
    }

    #[tokio::test]
    async fn a_resend_answers_the_same_for_every_name_and_mails_only_an_unverified_account() {
        let temp = TempDatabase::open();
        temp.db
            .setup_new_transaction(|conn| Ok(insert_user(conn, "bob")))
            .unwrap();
        let (mailer, sent) = Mailer::capture();
        let app = set_up_routes().with_state(temp.state(mailer));

        let mut answers = Vec::new();
        for name in ["bob", "nobody"] {
            let (status, _, body) = call(
                &app,
                post_json(
                    "/api/v1/auth/verify_email/resend",
                    serde_json::json!({ "username_or_email": name }),
                ),
            )
            .await;
            answers.push((status, body));
        }
        assert_eq!(answers[0].0, StatusCode::ACCEPTED);
        assert_eq!(answers[0], answers[1], "the two answers must be identical");

        // The mail goes out in a background task; give it up to two seconds.
        for _ in 0..200 {
            if !sent.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].email, "bob@example.com");
    }

    /// A request with `Authorization: Bearer <token>` and, if given, a JSON body.
    fn authorized(
        method: &str,
        uri: &str,
        token: &str,
        body: Option<serde_json::Value>,
    ) -> Request<Body> {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"));
        match body {
            Some(body) => request
                .header("content-type", "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap()
    }

    /// Every route except `/health`, the public `/api/v1/auth/…` routes and the catalog's reads
    /// (`/api/v1/locales…`). A route added to the table belongs here too, so this test proves it
    /// can't be reached without a login.
    const GUARDED_ROUTES: &[(&str, &str)] = &[
        ("GET", "/api/v1/libraries"),
        ("POST", "/api/v1/libraries"),
        ("PATCH", "/api/v1/libraries/1"),
        ("DELETE", "/api/v1/libraries/1"),
        ("GET", "/api/v1/libraries/1/series"),
        ("PUT", "/api/v1/libraries/1/series/1"),
        ("DELETE", "/api/v1/libraries/1/series/1"),
        ("PUT", "/api/v1/libraries/1/cover"),
        ("DELETE", "/api/v1/libraries/1/cover"),
        ("GET", "/api/v1/series"),
        ("PATCH", "/api/v1/series/1"),
        ("DELETE", "/api/v1/series/1"),
        ("GET", "/api/v1/series/1/books"),
        ("PUT", "/api/v1/series/1/cover"),
        ("DELETE", "/api/v1/series/1/cover"),
        ("PUT", "/api/v1/series/1/reading_mode"),
        ("GET", "/api/v1/books/1"),
        ("GET", "/api/v1/books/1/file"),
        ("GET", "/api/v1/books/1/pages"),
        ("GET", "/api/v1/books/1/pages/1"),
        ("GET", "/api/v1/books/1/progress"),
        ("PATCH", "/api/v1/books/1/progress"),
        ("GET", "/api/v1/covers/book/1"),
        ("POST", "/api/v1/scan"),
        ("GET", "/api/v1/scan/status"),
        ("GET", "/api/v1/directories"),
        ("POST", "/api/v1/directories"),
        ("DELETE", "/api/v1/directories/1"),
        ("POST", "/api/v1/directories/1/scan"),
        ("POST", "/api/v1/metadata/scan"),
        ("GET", "/api/v1/search"),
        ("GET", "/api/v1/account"),
        ("PUT", "/api/v1/account/password"),
        ("PUT", "/api/v1/account/username"),
        ("PUT", "/api/v1/account/email"),
        ("POST", "/api/v1/account/email/confirm"),
        ("GET", "/api/v1/account/settings"),
        ("PUT", "/api/v1/account/settings"),
        ("GET", "/api/v1/account/totp"),
        ("POST", "/api/v1/account/totp"),
        ("POST", "/api/v1/account/totp/confirm"),
        ("POST", "/api/v1/account/totp/disable"),
        ("POST", "/api/v1/account/totp/recovery_codes"),
        ("GET", "/api/v1/users"),
        ("PATCH", "/api/v1/users/1"),
    ];

    #[tokio::test]
    async fn the_catalog_needs_no_session_and_ignores_a_dead_one() {
        let temp = TempDatabase::open();
        temp.db
            .setup_new_transaction(|conn| {
                insert_texts(
                    conn,
                    "frontend",
                    "test",
                    "title",
                    &[("en", "Title"), ("de", "Titel")],
                );
                Ok(())
            })
            .unwrap();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        for token in [None, Some("not-a-session")] {
            let request = |uri: &str| match token {
                Some(token) => authorized("GET", uri, token, None),
                None => bare("GET", uri),
            };

            let (status, _, body) = call(&app, request("/api/v1/locales")).await;
            assert_eq!(status, StatusCode::OK, "{token:?}");
            assert_eq!(
                json(&body),
                serde_json::json!([
                    {"code": "en", "display_name": "English", "icon": null, "is_default": true},
                    {"code": "de", "display_name": "Deutsch", "icon": null, "is_default": false},
                ])
            );

            let uri = "/api/v1/locales/de/translations?sections=test";
            let (status, _, body) = call(&app, request(uri)).await;
            assert_eq!(status, StatusCode::OK, "{token:?}");
            assert_eq!(
                json(&body),
                serde_json::json!({ "test": { "title": "Titel" } })
            );
        }
    }

    #[tokio::test]
    async fn a_language_the_server_doesnt_offer_answers_404_json() {
        let temp = TempDatabase::open();
        temp.db
            .setup_new_transaction(|conn| {
                conn.execute("UPDATE locales SET enabled = FALSE WHERE code = 'de'", [])
                    .unwrap();
                Ok(())
            })
            .unwrap();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        for uri in [
            "/api/v1/locales/fr/translations",
            "/api/v1/locales/de/translations",
        ] {
            let (status, _, body) = call(&app, bare("GET", uri)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(
                json(&body),
                serde_json::json!({ "error": "not_found" }),
                "{uri}"
            );
        }
    }

    #[tokio::test]
    async fn a_catalog_query_the_server_cant_read_answers_400_json() {
        let (status, body) = json_answer(bare(
            "GET",
            "/api/v1/locales/en/translations?sections=errors&sections=email",
        ))
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, serde_json::json!({ "error": "invalid_request" }));
    }

    fn catalog_request(uri: &str, if_none_match: Option<&str>) -> Request<Body> {
        let mut request = Request::get(uri);
        if let Some(tags) = if_none_match {
            request = request.header("if-none-match", tags);
        }
        request.body(Body::empty()).unwrap()
    }

    async fn etag_of_answer(app: &Router, uri: &str) -> String {
        let (status, headers, _) = call(app, catalog_request(uri, None)).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        headers["etag"].to_str().unwrap().to_string()
    }

    /// Several keys in two namespaces, so a map that iterated in a random order would show up as
    /// a changing ETag; `test.untranslated` has no German text.
    fn add_catalog_fixture(temp: &TempDatabase) {
        temp.db
            .setup_new_transaction(|conn| {
                for name in ["title", "subtitle", "body", "footer"] {
                    let german = format!("{name} auf Deutsch");
                    insert_texts(
                        conn,
                        "frontend",
                        "test",
                        name,
                        &[("en", name), ("de", german.as_str())],
                    );
                }
                insert_texts(
                    conn,
                    "frontend",
                    "test",
                    "untranslated",
                    &[("en", "Only English")],
                );
                insert_texts(conn, "frontend", "other", "a", &[("en", "A")]);
                Ok(())
            })
            .unwrap();
    }

    #[tokio::test]
    async fn the_catalog_answers_304_to_an_app_that_has_its_etag() {
        let temp = TempDatabase::open();
        add_catalog_fixture(&temp);
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let uri = "/api/v1/locales/de/translations?sections=test,other";

        let (status, headers, body) = call(&app, catalog_request(uri, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["content-type"], "application/json");
        assert_eq!(headers["cache-control"], "no-cache");
        assert_eq!(json(&body)["test"]["title"], "title auf Deutsch");
        let etag = headers["etag"].to_str().unwrap().to_string();
        assert_eq!(
            etag_of_answer(&app, uri).await,
            etag,
            "the same texts give the same ETag"
        );

        let weak = format!("W/{etag}");
        let listed = format!("\"stale\", {etag}");
        for tags in [etag.as_str(), weak.as_str(), listed.as_str(), "*"] {
            let (status, headers, body) = call(&app, catalog_request(uri, Some(tags))).await;
            assert_eq!(status, StatusCode::NOT_MODIFIED, "{tags}");
            assert_eq!(headers["etag"], etag.as_str(), "{tags}");
            assert_eq!(headers["cache-control"], "no-cache", "{tags}");
            assert!(body.is_empty(), "{tags}");
        }

        let (status, _, body) = call(&app, catalog_request(uri, Some("\"stale\""))).await;
        assert_eq!(status, StatusCode::OK, "another tag gets the texts");
        assert_eq!(json(&body)["other"]["a"], "A");
    }

    #[tokio::test]
    async fn the_etag_changes_exactly_when_the_answer_does() {
        let temp = TempDatabase::open();
        add_catalog_fixture(&temp);
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let change = |sql: &str| {
            temp.db
                .setup_new_transaction(|conn| {
                    conn.execute(sql, []).unwrap();
                    Ok(())
                })
                .unwrap();
        };
        let german = "/api/v1/locales/de/translations?sections=test";

        let first = etag_of_answer(&app, german).await;
        change("UPDATE translations SET value = 'Changed' WHERE value = 'Only English'");
        let after_english_edit = etag_of_answer(&app, german).await;
        assert_ne!(
            after_english_edit, first,
            "German shows this English text as its fallback"
        );

        change("UPDATE translations SET value = 'Neu' WHERE value = 'title auf Deutsch'");
        assert_ne!(etag_of_answer(&app, german).await, after_english_edit);
        change("UPDATE translations SET value = 'title auf Deutsch' WHERE value = 'Neu'");
        assert_eq!(
            etag_of_answer(&app, german).await,
            after_english_edit,
            "the same texts again give the same ETag again"
        );

        let both = "/api/v1/locales/de/translations?sections=test,other";
        assert_ne!(
            etag_of_answer(&app, both).await,
            etag_of_answer(&app, german).await,
            "another selection is another answer"
        );
    }

    #[tokio::test]
    async fn every_route_but_health_and_auth_needs_a_session() {
        let temp = TempDatabase::open();
        let user = temp
            .db
            .setup_new_transaction(|conn| Ok(insert_user(conn, "alice")))
            .unwrap();
        add_session(
            &temp,
            user,
            "d1",
            "expired",
            "expired-refresh",
            unix_now() - 1,
        );
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));

        for &(method, uri) in GUARDED_ROUTES {
            for token in [None, Some("expired")] {
                let request = match token {
                    Some(token) => authorized(method, uri, token, None),
                    None => bare(method, uri),
                };
                let (status, _, body) = call(&app, request).await;
                assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri} {token:?}");
                assert_eq!(
                    json(&body),
                    serde_json::json!({ "error": "unauthorized" }),
                    "{method} {uri}"
                );
            }
        }
    }

    #[tokio::test]
    async fn only_an_admin_scans_or_deletes_a_library() {
        let temp = TempDatabase::open();
        let member = logged_in(&temp, "bob", "member");
        let admin = logged_in(&temp, "alice", "admin");
        let library = temp
            .db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO library (library_name, library_type) VALUES ('books', 'books')",
                    [],
                )
                .unwrap();
                Ok(conn.last_insert_rowid())
            })
            .unwrap();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let delete = format!("/api/v1/libraries/{library}");
        let libraries = |temp: &TempDatabase| -> i64 {
            temp.db
                .setup_new_transaction(|conn| {
                    Ok(conn
                        .query_row("SELECT COUNT(*) FROM library", [], |row| row.get(0))
                        .unwrap())
                })
                .unwrap()
        };

        for (method, uri) in [("POST", "/api/v1/scan"), ("DELETE", delete.as_str())] {
            let (status, _, body) = call(&app, authorized(method, uri, &member, None)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
            assert_eq!(json(&body), serde_json::json!({ "error": "forbidden" }));
        }
        assert_eq!(libraries(&temp), 1, "a member deletes nothing");

        // An empty folder, so the scan is over before the answer comes.
        let folder = TempDir::new();
        std::fs::create_dir_all(folder.path()).unwrap();
        temp.db
            .setup_new_transaction(|conn| {
                conn.execute(
                    "INSERT INTO directories (path) VALUES (?1)",
                    [folder.path().to_string_lossy()],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
        let (status, _, _) = call(&app, authorized("POST", "/api/v1/scan", &admin, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(libraries(&temp), 1, "a scan makes no library");

        let (status, _, _) = call(&app, authorized("DELETE", &delete, &admin, None)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(libraries(&temp), 0);
    }

    #[tokio::test]
    async fn only_an_admin_manages_users_and_a_new_role_counts_at_once() {
        let temp = TempDatabase::open();
        let admin = logged_in(&temp, "alice", "admin");
        let member = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let list_as = |token: &str| authorized("GET", "/api/v1/users", token, None);
        let patch = |id: &str, body: serde_json::Value| {
            authorized("PATCH", &format!("/api/v1/users/{id}"), &admin, Some(body))
        };

        let (status, _, body) = call(&app, list_as(&member)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json(&body), serde_json::json!({ "error": "forbidden" }));

        let (status, _, body) = call(&app, list_as(&admin)).await;
        assert_eq!(status, StatusCode::OK);
        let text = String::from_utf8(body.clone()).unwrap();
        assert!(
            !text.contains("password") && !text.contains("not-a-hash"),
            "no password hash, not even its key: {text}"
        );
        let users = json(&body);
        let user = |name: &str| {
            users
                .as_array()
                .unwrap()
                .iter()
                .find(|user| user["username"] == name)
                .unwrap()
                .clone()
        };
        let (alice, bob) = (user("alice"), user("bob"));
        let mut keys: Vec<&String> = bob.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "created_at",
                "email",
                "email_verified",
                "id",
                "last_login",
                "role",
                "username"
            ]
        );
        assert_eq!(bob["role"], "member");
        assert!(bob["created_at"].is_i64());
        let (alice, bob) = (alice["id"].to_string(), bob["id"].to_string());

        let (status, _, body) =
            call(&app, patch(&bob, serde_json::json!({ "role": "admin" }))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&body)["role"], "admin");
        let (status, _, _) = call(&app, list_as(&member)).await;
        assert_eq!(status, StatusCode::OK, "promoted on the very next request");

        let (status, _, body) =
            call(&app, patch(&bob, serde_json::json!({ "role": "member" }))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&body)["role"], "member");
        let (status, _, _) = call(&app, list_as(&member)).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "demoted on the very next request"
        );

        let (status, _, body) =
            call(&app, patch(&alice, serde_json::json!({ "role": "member" }))).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(json(&body), serde_json::json!({ "error": "last_admin" }));

        for (id, change, expected) in [
            (
                "abc",
                serde_json::json!({ "role": "admin" }),
                StatusCode::BAD_REQUEST,
            ),
            (
                bob.as_str(),
                serde_json::json!({ "role": "owner" }),
                StatusCode::BAD_REQUEST,
            ),
            (bob.as_str(), serde_json::json!({}), StatusCode::BAD_REQUEST),
            (
                bob.as_str(),
                serde_json::json!({ "email_verified": false }),
                StatusCode::BAD_REQUEST,
            ),
            (
                "999",
                serde_json::json!({ "role": "admin" }),
                StatusCode::NOT_FOUND,
            ),
        ] {
            let (status, _, _) = call(&app, patch(id, change.clone())).await;
            assert_eq!(status, expected, "{id} {change}");
        }
    }

    #[tokio::test]
    async fn the_code_steps_need_a_live_challenge() {
        let temp = TempDatabase::open();
        let user = temp
            .db
            .setup_new_transaction(|conn| Ok(insert_user(conn, "alice")))
            .unwrap();
        add_session(
            &temp,
            user,
            "d1",
            "session-1",
            "refresh-1",
            unix_now() + 600,
        );
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let body = serde_json::json!({ "code": "123456" });

        for uri in [
            "/api/v1/auth/login/totp",
            "/api/v1/auth/login/recovery_code",
        ] {
            for token in [None, Some("unknown"), Some("session-1")] {
                let request = match token {
                    Some(token) => authorized("POST", uri, token, Some(body.clone())),
                    None => post_json(uri, body.clone()),
                };
                let (status, _, answer) = call(&app, request).await;
                assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri} {token:?}");
                assert_eq!(
                    json(&answer),
                    serde_json::json!({ "error": "unauthorized" })
                );
            }
        }
    }

    async fn add_alice_with_password(temp: &TempDatabase) {
        let hash = hash_secret("correct horse battery".to_string())
            .await
            .unwrap();
        temp.db
            .setup_new_transaction(move |conn| {
                conn.execute(
                    "INSERT INTO users (username, password_hash, email, email_verified) \
                     VALUES ('alice', ?1, 'alice@example.com', 1)",
                    [hash],
                )
                .unwrap();
                Ok(())
            })
            .unwrap();
    }

    fn log_in_as_alice() -> Request<Body> {
        post_json(
            "/api/v1/auth/login",
            serde_json::json!({
                "username_or_email": "alice",
                "password": "correct horse battery",
                "device_name": "phone",
                "platform": "ios",
            }),
        )
    }

    /// Switches 2FA on over HTTP; answers the authenticator app and the recovery codes.
    async fn enroll_over_http(app: &Router, session: &str) -> (totp_rs::Totp, Vec<String>) {
        let (_, _, body) = call(
            app,
            authorized("POST", "/api/v1/account/totp", session, None),
        )
        .await;
        let authenticator = totp_rs::Builder::new()
            .with_secret(
                totp_rs::Secret::try_from_base32(json(&body)["secret_base32"].as_str().unwrap())
                    .unwrap(),
            )
            .build()
            .unwrap();
        let confirmation = serde_json::json!({
            "code": authenticator.generate_current().to_string(),
            "password": "correct horse battery",
        });
        let (status, _, body) = call(
            app,
            authorized(
                "POST",
                "/api/v1/account/totp/confirm",
                session,
                Some(confirmation),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let codes = serde_json::from_value(json(&body)["recovery_codes"].clone()).unwrap();
        (authenticator, codes)
    }

    /// Logs Alice in with her password; answers the challenge token of the code step.
    async fn challenge_token(app: &Router) -> String {
        let (status, _, body) = call(app, log_in_as_alice()).await;
        assert_eq!(status, StatusCode::OK);
        let body = json(&body);
        assert_eq!(body["status"], "totp_required");
        body["challenge_token"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn logging_in_with_2fa_and_turning_it_off_over_http() {
        let temp = TempDatabase::open();
        add_alice_with_password(&temp).await;
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let (_, _, body) = call(&app, log_in_as_alice()).await;
        let setup_session = json(&body)["session_token"].as_str().unwrap().to_string();
        let (authenticator, recovery_codes) = enroll_over_http(&app, &setup_session).await;

        let challenge = challenge_token(&app).await;
        let (status, _, _) = call(&app, guarded(Some(&format!("Bearer {challenge}")))).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "a challenge token is no session"
        );
        // The confirm above used the current time step, so this sends the next one; the check
        // accepts one step of clock drift.
        let next_step = authenticator.generate(unix_now() as u64 + 30).to_string();
        let (status, headers, body) = call(
            &app,
            authorized(
                "POST",
                "/api/v1/auth/login/totp",
                &challenge,
                Some(serde_json::json!({ "code": next_step })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get("cache-control").unwrap(), "no-store");
        assert_eq!(json(&body)["status"], "ok");
        let session = json(&body)["session_token"].as_str().unwrap().to_string();
        let (status, _, _) = call(&app, guarded(Some(&format!("Bearer {session}")))).await;
        assert_eq!(status, StatusCode::OK);

        let challenge = challenge_token(&app).await;
        let (status, headers, body) = call(
            &app,
            authorized(
                "POST",
                "/api/v1/auth/login/recovery_code",
                &challenge,
                Some(serde_json::json!({ "code": recovery_codes[0] })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get("cache-control").unwrap(), "no-store");
        assert_eq!(json(&body)["status"], "ok");

        let turn_off = serde_json::json!({
            "password": "correct horse battery",
            "code": recovery_codes[1],
        });
        let (status, _, _) = call(
            &app,
            authorized(
                "POST",
                "/api/v1/account/totp/disable",
                &session,
                Some(turn_off),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _, _) = call(&app, guarded(Some(&format!("Bearer {session}")))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "every session ended");
        let (_, _, body) = call(&app, log_in_as_alice()).await;
        assert_eq!(json(&body)["status"], "ok", "no code step any more");
    }

    #[tokio::test]
    async fn renewing_the_recovery_codes_over_http() {
        let temp = TempDatabase::open();
        add_alice_with_password(&temp).await;
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let (_, _, body) = call(&app, log_in_as_alice()).await;
        let session = json(&body)["session_token"].as_str().unwrap().to_string();
        let (authenticator, old_codes) = enroll_over_http(&app, &session).await;

        // The confirm used the current time step, so this sends the next one.
        let renewal = serde_json::json!({
            "password": "correct horse battery",
            "code": authenticator.generate(unix_now() as u64 + 30).to_string(),
        });
        let (status, headers, body) = call(
            &app,
            authorized(
                "POST",
                "/api/v1/account/totp/recovery_codes",
                &session,
                Some(renewal),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get("cache-control").unwrap(), "no-store");
        let new_codes: Vec<String> =
            serde_json::from_value(json(&body)["recovery_codes"].clone()).unwrap();
        assert_eq!(new_codes.len(), 10);

        let challenge = challenge_token(&app).await;
        let with_code = |code: &str| {
            authorized(
                "POST",
                "/api/v1/auth/login/recovery_code",
                &challenge,
                Some(serde_json::json!({ "code": code })),
            )
        };
        let (status, _, body) = call(&app, with_code(&old_codes[0])).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "an old code");
        assert_eq!(
            json(&body),
            serde_json::json!({ "error": "invalid_or_expired_code" })
        );
        let (status, _, body) = call(&app, with_code(&new_codes[0])).await;
        assert_eq!(status, StatusCode::OK, "a new code");
        assert_eq!(json(&body)["status"], "ok");
    }

    #[tokio::test]
    async fn changing_the_password_over_http() {
        let temp = TempDatabase::open();
        add_alice_with_password(&temp).await;
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let (_, _, body) = call(&app, log_in_as_alice()).await;
        let this_device = json(&body)["session_token"].as_str().unwrap().to_string();
        let (_, _, body) = call(&app, log_in_as_alice()).await;
        let other_device = json(&body)["session_token"].as_str().unwrap().to_string();

        let change = serde_json::json!({
            "current_password": "correct horse battery",
            "new_password": "battery staple horse",
            "new_password_repeat": "battery staple horse",
        });
        let (status, _, body) = call(
            &app,
            authorized(
                "PUT",
                "/api/v1/account/password",
                &this_device,
                Some(change),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_empty());

        let (status, _, _) = call(&app, guarded(Some(&format!("Bearer {this_device}")))).await;
        assert_eq!(status, StatusCode::OK, "this session stays");
        let (status, _, _) = call(&app, guarded(Some(&format!("Bearer {other_device}")))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "the other device's ended");
        let (status, _, _) = call(&app, log_in_as_alice()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "the old password");
        let with_the_new_password = post_json(
            "/api/v1/auth/login",
            serde_json::json!({
                "username_or_email": "alice",
                "password": "battery staple horse",
                "device_name": "phone",
                "platform": "ios",
            }),
        );
        let (status, _, _) = call(&app, with_the_new_password).await;
        assert_eq!(status, StatusCode::OK, "the new password");
    }

    #[tokio::test]
    async fn changing_the_username_over_http() {
        let temp = TempDatabase::open();
        add_alice_with_password(&temp).await;
        temp.db
            .setup_new_transaction(|conn| Ok(insert_user(conn, "bob")))
            .unwrap();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let (_, _, body) = call(&app, log_in_as_alice()).await;
        let session = json(&body)["session_token"].as_str().unwrap().to_string();
        let rename = |name: &str| {
            authorized(
                "PUT",
                "/api/v1/account/username",
                &session,
                Some(serde_json::json!({ "new_username": name })),
            )
        };

        let (status, _, body) = call(&app, rename("BOB")).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            json(&body),
            serde_json::json!({ "error": "username_taken" })
        );
        let (status, _, body) = call(&app, rename("al")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json(&body),
            serde_json::json!({ "error": "invalid_username" })
        );
        let (status, _, body) = call(&app, rename("alicia")).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_empty());

        let as_alicia = post_json(
            "/api/v1/auth/login",
            serde_json::json!({
                "username_or_email": "alicia",
                "password": "correct horse battery",
                "device_name": "phone",
                "platform": "ios",
            }),
        );
        let (status, _, _) = call(&app, as_alicia).await;
        assert_eq!(status, StatusCode::OK, "the new name logs in");
    }

    fn log_in_as(email: &str) -> Request<Body> {
        post_json(
            "/api/v1/auth/login",
            serde_json::json!({
                "username_or_email": email,
                "password": "correct horse battery",
                "device_name": "phone",
                "platform": "ios",
            }),
        )
    }

    #[tokio::test]
    async fn changing_the_email_over_http() {
        let temp = TempDatabase::open();
        add_alice_with_password(&temp).await;
        let (mailer, sent) = Mailer::capture();
        let app = set_up_routes().with_state(temp.state(mailer));
        let (_, _, body) = call(&app, log_in_as_alice()).await;
        let session = json(&body)["session_token"].as_str().unwrap().to_string();

        let change = serde_json::json!({
            "new_email": "alice.new@example.com",
            "password": "correct horse battery",
        });
        let (status, _, body) = call(
            &app,
            authorized("PUT", "/api/v1/account/email", &session, Some(change)),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(body.is_empty());

        // The mail goes out in a background task; give it up to two seconds.
        for _ in 0..200 {
            if !sent.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let code = {
            let sent = sent.lock().unwrap();
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0].email, "alice.new@example.com");
            code_in(&sent[0])
        };
        let (status, _, body) = call(
            &app,
            authorized(
                "POST",
                "/api/v1/account/email/confirm",
                &session,
                Some(serde_json::json!({ "code": code })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_empty());

        let (status, _, _) = call(&app, log_in_as("alice.new@example.com")).await;
        assert_eq!(status, StatusCode::OK, "the new address logs in");
        let (status, _, _) = call(&app, log_in_as("alice@example.com")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "the old one doesn't");
    }

    #[tokio::test]
    async fn changing_the_email_without_mail_over_http() {
        let temp = TempDatabase::open();
        add_alice_with_password(&temp).await;
        temp.db
            .setup_new_transaction(|conn| Ok(insert_user(conn, "bob")))
            .unwrap();
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let (_, _, body) = call(&app, log_in_as_alice()).await;
        let session = json(&body)["session_token"].as_str().unwrap().to_string();
        let change_to = |email: &str| {
            authorized(
                "PUT",
                "/api/v1/account/email",
                &session,
                Some(serde_json::json!({
                    "new_email": email,
                    "password": "correct horse battery",
                })),
            )
        };

        let (status, _, body) = call(&app, change_to("bob@example.com")).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(json(&body), serde_json::json!({ "error": "email_taken" }));
        let (status, _, body) = call(&app, change_to("ALICE@example.com")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json(&body),
            serde_json::json!({ "error": "email_unchanged" })
        );
        let (status, _, _) = call(&app, change_to("alice.new@example.com")).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "applied at once");

        let (status, _, _) = call(&app, log_in_as("alice.new@example.com")).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn reading_and_replacing_the_settings_over_http() {
        let temp = TempDatabase::open();
        let alice = logged_in(&temp, "alice", "member");
        let bob = logged_in(&temp, "bob", "member");
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let read_as = |token: &str| authorized("GET", "/api/v1/account/settings", token, None);
        let replace = |settings: serde_json::Value| {
            authorized("PUT", "/api/v1/account/settings", &alice, Some(settings))
        };

        let (status, _, body) = call(&app, read_as(&alice)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json(&body),
            serde_json::json!({ "locale": null }),
            "the defaults"
        );

        let (status, _, body) = call(&app, replace(serde_json::json!({ "locale": "de" }))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&body), serde_json::json!({ "locale": "de" }));
        let (_, _, body) = call(&app, read_as(&alice)).await;
        assert_eq!(json(&body), serde_json::json!({ "locale": "de" }));
        let (_, _, body) = call(&app, read_as(&bob)).await;
        assert_eq!(
            json(&body),
            serde_json::json!({ "locale": null }),
            "bob's own"
        );

        for (settings, key) in [
            (serde_json::json!({ "locale": "pt-BR" }), "invalid_locale"),
            (serde_json::json!({ "locale": "german" }), "invalid_locale"),
            (serde_json::json!({ "locale": "" }), "invalid_locale"),
            (serde_json::json!({ "locale": 49 }), "invalid_request"),
        ] {
            let (status, _, body) = call(&app, replace(settings.clone())).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{settings}");
            assert_eq!(
                json(&body),
                serde_json::json!({ "error": key }),
                "{settings}"
            );
        }
        let (_, _, body) = call(&app, read_as(&alice)).await;
        assert_eq!(
            json(&body),
            serde_json::json!({ "locale": "de" }),
            "unchanged"
        );

        let (status, _, body) = call(&app, replace(serde_json::json!({ "locale": null }))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&body), serde_json::json!({ "locale": null }));
        let (_, _, body) = call(&app, read_as(&alice)).await;
        assert_eq!(json(&body), serde_json::json!({ "locale": null }), "reset");
    }

    #[tokio::test]
    async fn setting_up_2fa_over_http() {
        let temp = TempDatabase::open();
        add_alice_with_password(&temp).await;
        let app = set_up_routes().with_state(temp.state(Mailer::Disabled));
        let log_in = log_in_as_alice;
        let (_, _, body) = call(&app, log_in()).await;
        let session = json(&body)["session_token"].as_str().unwrap().to_string();
        let (_, _, body) = call(&app, log_in()).await;
        let other_device = json(&body)["session_token"].as_str().unwrap().to_string();
        let status_of_2fa = || authorized("GET", "/api/v1/account/totp", &session, None);

        let (status, _, body) = call(&app, status_of_2fa()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json(&body),
            serde_json::json!({ "enabled": false, "recovery_codes_left": 0 })
        );

        let (status, headers, body) = call(
            &app,
            authorized("POST", "/api/v1/account/totp", &session, None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get("cache-control").unwrap(), "no-store");
        let setup = json(&body);
        assert!(setup["otpauth_url"]
            .as_str()
            .unwrap()
            .starts_with("otpauth://totp/Bookserver:alice?secret="));
        let authenticator = totp_rs::Builder::new()
            .with_secret(
                totp_rs::Secret::try_from_base32(setup["secret_base32"].as_str().unwrap()).unwrap(),
            )
            .build()
            .unwrap();

        let confirmation = serde_json::json!({
            "code": authenticator.generate_current().to_string(),
            "password": "correct horse battery",
        });
        let (status, headers, body) = call(
            &app,
            authorized(
                "POST",
                "/api/v1/account/totp/confirm",
                &session,
                Some(confirmation),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get("cache-control").unwrap(), "no-store");
        assert_eq!(json(&body)["recovery_codes"].as_array().unwrap().len(), 10);

        let (status, _, _) = call(&app, guarded(Some(&format!("Bearer {other_device}")))).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "the other device's session ended"
        );
        let (_, _, body) = call(&app, status_of_2fa()).await;
        assert_eq!(
            json(&body),
            serde_json::json!({ "enabled": true, "recovery_codes_left": 10 })
        );
        let (status, _, body) = call(&app, log_in()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&body)["status"], "totp_required");
    }

    #[tokio::test]
    async fn a_resend_without_email_configured_answers_503() {
        let (status, body) = json_answer(post_json(
            "/api/v1/auth/verify_email/resend",
            serde_json::json!({ "username_or_email": "bob" }),
        ))
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body, serde_json::json!({ "error": "email_not_configured" }));
    }
}
