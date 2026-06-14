pub(super) fn lock_store<'a>(
    state: &'a State<'_, AppState>,
) -> Result<std::sync::MutexGuard<'a, RepositoryStore>, String> {
    lock_app_store(state)
}

pub(super) fn lock_app_store(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, RepositoryStore>, String> {
    state
        .store
        .lock()
        .map_err(|_| "Repository store lock is poisoned.".to_string())
}

#[allow(dead_code)]
pub(super) fn source_has_http(sources: &[SourceUri]) -> bool {
    sources
        .iter()
        .any(|source| matches!(source, SourceUri::Http { .. }))
}
