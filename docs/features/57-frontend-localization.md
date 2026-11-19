# Frontend localization

State: completed

Every UI string goes through `context.t("key")` from the first screen, even while only English exists,
so adding a language later is catalog data instead of a rework. The client fetches the
[catalog](13-internationalization.md) once, caches it, and re-checks it by version.

## Build
- [x] 2 h · `context.t("namespace.key")` lookup for every UI string, from the first screen on [FE]
- [x] 1 h · Interpolation: `context.t("key", {"name": value})` fills each `{name}` placeholder, matched by the backend's pattern (`{[a-z0-9_]+}`); a value not given leaves its placeholder visible [FE]
- [x] 1 h · Plural forms — one key per form (`unread.one` / `unread.other`), picked with the language's plural rules, `.other` when a form is missing [FE]
- [x] 2 h · The seed accepts the plural forms a language has beyond English's — `zero`, `two`, `few`, `many` — next to a key English writes as `.one` / `.other`, with English `.other`'s placeholders; today it stops the server on any key English lacks [BE]
- [x] 2 h · Load the catalog from the server at runtime with the app's own `Catalog.fromJson` — no i18n library — and swap it in under every screen; a key the server's catalog lacks falls back to the built-in English [FE]
- [x] 1 h · Ship the English catalog built into the app, so a cold or offline start still renders [FE]
- [x] 1 h · Cache the fetched catalog [FE]
- [x] 1 h · Re-check the cached catalog with `If-None-Match` and its ETag — on app start and whenever the language is picked again — instead of downloading it every time; a `304` keeps the cached copy [FE]
- [x] 1 h · Pick the language, again whenever the server or the sign-in changes: the user's setting ([31](31-per-user-settings.md)), else the first of the browser's or OS's languages the server offers — the same language and region, else the language alone, else the server's first code of that language — else the default from `GET /api/v1/locales` [FE]
- [x] 1 h · Material's own texts — the Back tooltip, the Cut/Copy/Paste menu — follow the catalog's language where `material_ui` has them, else English [FE]
- [x] 0.5 h · Check by hand: the first screen shows no literal UI strings
