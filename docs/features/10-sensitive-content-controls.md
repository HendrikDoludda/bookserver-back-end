# Sensitive content flag

State: not started

An admin can mark a library or series as sensitive. Its titles are then never sent to an online
metadata service ([09](09-metadata-enrichment.md) owns that check). Further protections are
separate, optional features: [hidden from default views](67-hide-sensitive-items.md),
[per-user visibility](68-sensitive-per-user-visibility.md),
[blurred covers](69-blurred-sensitive-covers.md) and a
[PIN or re-login to open](62-sensitive-pin-or-reauth.md).

## Build
- [ ] ? h · Admin-only toggle endpoint for a library and a series (`AdminSession`; `Update` already works for both tables), adding `IsSensitive` to `LibraryDatabaseColumns` and `SeriesDatabaseColumns` — the columns and model fields come with 09's migration 0011 [BE]
- [ ] ? h · Effective sensitivity: an item is sensitive if its series or any of its libraries is — through the query 09's lookup gate uses, so every endpoint gives the same answer for an item [BE]
- [ ] ? h · Sensitive toggle on the library and series screens — the same endpoint from the main frontend and the admin web view [FE]
