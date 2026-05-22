use crate::cache_path;
use anyhow::{bail, Context, Result};
use blake3::Hasher;
use git2::Repository;
use serde_json::{json, Value};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const TOOL: &str = "regret";

#[derive(Debug, Clone)]
pub(crate) struct RegretPaths {
    pub(crate) cmdrvl_root: PathBuf,
    pub(crate) repo_id: String,
    pub(crate) config_file: PathBuf,
    pub(crate) cache_dir: PathBuf,
    pub(crate) state_dir: PathBuf,
    pub(crate) lock_file: PathBuf,
    pub(crate) commit_template: PathBuf,
    pub(crate) adoption_doc: PathBuf,
    pub(crate) agent_snippets_dir: PathBuf,
    pub(crate) ci_dir: PathBuf,
    pub(crate) hooks_dir: PathBuf,
    pub(crate) commit_msg_hook: PathBuf,
    pub(crate) legacy_dir: PathBuf,
    migration_log: PathBuf,
    deprecation_notices: PathBuf,
}

pub(crate) fn resolve_for_repo(repo_root: &Path) -> RegretPaths {
    resolve_for_repo_from_env(repo_root, |key| std::env::var_os(key))
}

fn resolve_for_repo_from_env<F>(repo_root: &Path, get_env: F) -> RegretPaths
where
    F: Fn(&str) -> Option<OsString> + Copy,
{
    let root = cmdrvl_root_from_env(get_env);
    let repo_id = compute_repo_id(repo_root);
    let repo_config_dir = root
        .join("config")
        .join("regret")
        .join("repos")
        .join(&repo_id);
    let cache_dir = root
        .join("cache")
        .join("regret")
        .join("repos")
        .join(&repo_id);
    let state_dir = root
        .join("state")
        .join("regret")
        .join("repos")
        .join(&repo_id);
    let lock_dir = root
        .join("locks")
        .join("regret")
        .join("repos")
        .join(&repo_id);
    let hooks_dir = state_dir.join("hooks");

    RegretPaths {
        cmdrvl_root: root.clone(),
        repo_id,
        config_file: repo_config_dir.join("config.toml"),
        cache_dir,
        lock_file: lock_dir.join("scan.lock"),
        commit_template: state_dir.join("commit-template.txt"),
        adoption_doc: state_dir.join("ADOPTION.md"),
        agent_snippets_dir: state_dir.join("agent-snippets"),
        ci_dir: state_dir.join("ci"),
        commit_msg_hook: hooks_dir.join("commit-msg"),
        hooks_dir,
        state_dir,
        legacy_dir: repo_root.join(".regret"),
        migration_log: root.join("migrations").join("applied.jsonl"),
        deprecation_notices: root.join("notices").join("deprecated-paths.jsonl"),
    }
}

pub(crate) fn migrate_legacy(paths: &RegretPaths) -> Result<()> {
    migrate_file(
        paths,
        "regret_config",
        &paths.legacy_dir.join("config.toml"),
        &paths.config_file,
    )?;
    migrate_file(
        paths,
        "regret_cache_db",
        &paths.legacy_dir.join("cache.db"),
        &paths.cache_dir.join("cache.db"),
    )?;
    migrate_file(
        paths,
        "regret_cache_db_wal",
        &paths.legacy_dir.join("cache.db-wal"),
        &paths.cache_dir.join("cache.db-wal"),
    )?;
    migrate_file(
        paths,
        "regret_cache_db_shm",
        &paths.legacy_dir.join("cache.db-shm"),
        &paths.cache_dir.join("cache.db-shm"),
    )?;
    migrate_file(
        paths,
        "regret_commit_template",
        &paths.legacy_dir.join("commit-template.txt"),
        &paths.commit_template,
    )?;
    migrate_file(
        paths,
        "regret_adoption_doc",
        &paths.legacy_dir.join("ADOPTION.md"),
        &paths.adoption_doc,
    )?;
    migrate_dir(
        paths,
        "regret_agent_snippets",
        &paths.legacy_dir.join("agent-snippets"),
        &paths.agent_snippets_dir,
    )?;
    migrate_dir(
        paths,
        "regret_ci",
        &paths.legacy_dir.join("ci"),
        &paths.ci_dir,
    )?;
    migrate_dir(
        paths,
        "regret_hooks",
        &paths.legacy_dir.join("hooks"),
        &paths.hooks_dir,
    )?;
    Ok(())
}

pub(crate) fn ensure_init_dirs(paths: &RegretPaths) -> Result<()> {
    cache_path::ensure_cache_dir(&paths.state_dir)?;
    cache_path::ensure_cache_dir(&paths.agent_snippets_dir)?;
    cache_path::ensure_cache_dir(&paths.ci_dir)?;
    cache_path::ensure_cache_dir(&paths.hooks_dir)?;
    Ok(())
}

pub(crate) fn is_initialized(paths: &RegretPaths) -> bool {
    paths.commit_template.exists() || paths.legacy_dir.join("commit-template.txt").exists()
}

pub(crate) fn display_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// Compute a stable repo_id from the git common dir (blake3 hex).
pub(crate) fn compute_repo_id(repo_path: &Path) -> String {
    let git_dir = Repository::discover(repo_path)
        .ok()
        .map(|repo| repo.path().to_path_buf())
        .unwrap_or_else(|| repo_path.to_path_buf());
    let common_dir = resolve_common_dir(&git_dir);
    let canonical = fs::canonicalize(&common_dir).unwrap_or(common_dir);

    let mut hasher = Hasher::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    hex::encode(digest.as_bytes())
}

fn resolve_common_dir(git_dir: &Path) -> PathBuf {
    let commondir_path = git_dir.join("commondir");
    match fs::read_to_string(&commondir_path) {
        Ok(contents) => {
            let trimmed = contents.trim();
            if trimmed.is_empty() {
                git_dir.to_path_buf()
            } else {
                let candidate = Path::new(trimmed);
                if candidate.is_absolute() {
                    candidate.to_path_buf()
                } else {
                    git_dir.join(candidate)
                }
            }
        }
        Err(_) => git_dir.to_path_buf(),
    }
}

fn migrate_file(
    paths: &RegretPaths,
    path_class: &str,
    legacy: &Path,
    canonical: &Path,
) -> Result<()> {
    if !legacy.exists() {
        return Ok(());
    }

    let metadata = fs::symlink_metadata(legacy)
        .with_context(|| format!("error: unable to stat legacy path {}", legacy.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        append_record_once(
            &paths.deprecation_notices,
            deprecation_record(
                path_class,
                legacy,
                canonical,
                "legacy_path_unsupported",
                "not_copied",
            ),
        )?;
        return Ok(());
    }

    if canonical.exists() {
        append_record_once(
            &paths.deprecation_notices,
            deprecation_record(
                path_class,
                legacy,
                canonical,
                "legacy_path_present",
                "canonical_preferred",
            ),
        )?;
        return Ok(());
    }

    prepare_parent(canonical)?;
    fs::copy(legacy, canonical).with_context(|| {
        format!(
            "error: unable to copy legacy path {} to {}",
            legacy.display(),
            canonical.display()
        )
    })?;
    harden_file(canonical, is_owner_executable(&metadata))?;

    append_record_once(
        &paths.migration_log,
        migration_record(path_class, legacy, canonical, "copied_legacy_to_canonical"),
    )?;
    append_record_once(
        &paths.deprecation_notices,
        deprecation_record(
            path_class,
            legacy,
            canonical,
            "legacy_path_migrated",
            "canonical_created",
        ),
    )?;
    Ok(())
}

fn migrate_dir(
    paths: &RegretPaths,
    path_class: &str,
    legacy: &Path,
    canonical: &Path,
) -> Result<()> {
    if !legacy.exists() {
        return Ok(());
    }

    let metadata = fs::symlink_metadata(legacy)
        .with_context(|| format!("error: unable to stat legacy path {}", legacy.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        append_record_once(
            &paths.deprecation_notices,
            deprecation_record(
                path_class,
                legacy,
                canonical,
                "legacy_path_unsupported",
                "not_copied",
            ),
        )?;
        return Ok(());
    }

    if canonical.exists() {
        append_record_once(
            &paths.deprecation_notices,
            deprecation_record(
                path_class,
                legacy,
                canonical,
                "legacy_path_present",
                "canonical_preferred",
            ),
        )?;
        return Ok(());
    }

    copy_dir_recursive(legacy, canonical)?;
    append_record_once(
        &paths.migration_log,
        migration_record(path_class, legacy, canonical, "copied_legacy_to_canonical"),
    )?;
    append_record_once(
        &paths.deprecation_notices,
        deprecation_record(
            path_class,
            legacy,
            canonical,
            "legacy_path_migrated",
            "canonical_created",
        ),
    )?;
    Ok(())
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<()> {
    cache_path::ensure_cache_dir(destination)?;

    for entry in fs::read_dir(source).with_context(|| {
        format!(
            "error: unable to read legacy directory {}",
            source.display()
        )
    })? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;

        if file_type.is_symlink() {
            bail!(
                "error: unsupported symlink in legacy regret directory: {}",
                source_path.display()
            );
        }
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            prepare_parent(&destination_path)?;
            let metadata = fs::metadata(&source_path)?;
            fs::copy(&source_path, &destination_path)?;
            harden_file(&destination_path, is_owner_executable(&metadata))?;
        } else {
            bail!(
                "error: unsupported non-regular entry in legacy regret directory: {}",
                source_path.display()
            );
        }
    }

    Ok(())
}

fn prepare_parent(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    cache_path::ensure_cache_dir(parent)
}

fn append_record_once(path: &Path, record: Value) -> Result<()> {
    if record_already_exists(path, &record)? {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        cache_path::ensure_cache_dir(parent)?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("error: unable to append {}", path.display()))?;
    writeln!(file, "{record}")?;
    file.flush()?;
    harden_file(path, false)
}

fn record_already_exists(path: &Path, record: &Value) -> Result<bool> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };

    Ok(contents.lines().any(|line| {
        let Ok(existing) = serde_json::from_str::<Value>(line) else {
            return false;
        };

        existing.get("tool") == record.get("tool")
            && existing.get("path_class") == record.get("path_class")
            && existing.get("source_path") == record.get("source_path")
            && existing.get("destination_path") == record.get("destination_path")
            && existing.get("action") == record.get("action")
    }))
}

fn migration_record(path_class: &str, source: &Path, destination: &Path, action: &str) -> Value {
    json!({
        "version": "cmdrvl.migration.v1",
        "tool": TOOL,
        "path_class": path_class,
        "source_path": source.display().to_string(),
        "destination_path": destination.display().to_string(),
        "action": action,
        "outcome": "ok",
        "secret_values_recorded": false
    })
}

fn deprecation_record(
    path_class: &str,
    source: &Path,
    destination: &Path,
    action: &str,
    outcome: &str,
) -> Value {
    json!({
        "version": "cmdrvl.deprecated_path_notice.v1",
        "tool": TOOL,
        "path_class": path_class,
        "source_path": source.display().to_string(),
        "destination_path": destination.display().to_string(),
        "action": action,
        "outcome": outcome,
        "secret_values_recorded": false
    })
}

fn cmdrvl_root_from_env<F>(get_env: F) -> PathBuf
where
    F: Fn(&str) -> Option<OsString> + Copy,
{
    if let Some(home) =
        non_empty_env(get_env, "HOME").or_else(|| non_empty_env(get_env, "USERPROFILE"))
    {
        return PathBuf::from(home).join(".cmdrvl");
    }

    PathBuf::from(".cmdrvl")
}

fn non_empty_env<F>(get_env: F, key: &str) -> Option<OsString>
where
    F: Fn(&str) -> Option<OsString> + Copy,
{
    let value = get_env(key)?;
    if value.is_empty() {
        return None;
    }
    if value.to_str().is_some_and(|value| value.trim().is_empty()) {
        return None;
    }
    Some(value)
}

#[cfg(unix)]
fn is_owner_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o100 != 0
}

#[cfg(not(unix))]
fn is_owner_executable(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn harden_file(path: &Path, executable: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if executable { 0o700 } else { 0o600 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn harden_file(_path: &Path, _executable: bool) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{migrate_legacy, resolve_for_repo_from_env};
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn env_for_home(home: &Path) -> impl Fn(&str) -> Option<OsString> + Copy + '_ {
        |key| match key {
            "HOME" => Some(home.as_os_str().to_owned()),
            "USERPROFILE" => None,
            _ => None,
        }
    }

    #[test]
    fn resolves_under_cmdrvl_root_and_repo_id() {
        let tmp = tempfile_dir("regret-paths-resolve");
        let repo = tmp.join("repo");
        let home = tmp.join("home");
        fs::create_dir_all(&repo).unwrap();

        let paths = resolve_for_repo_from_env(&repo, env_for_home(&home));

        assert_eq!(paths.cmdrvl_root, home.join(".cmdrvl"));
        assert_eq!(
            paths.config_file,
            home.join(".cmdrvl/config/regret/repos")
                .join(&paths.repo_id)
                .join("config.toml")
        );
        assert_eq!(
            paths.cache_dir,
            home.join(".cmdrvl/cache/regret/repos").join(&paths.repo_id)
        );
        assert_eq!(
            paths.state_dir,
            home.join(".cmdrvl/state/regret/repos").join(&paths.repo_id)
        );

        fs::remove_dir_all(tmp).ok();
    }

    #[test]
    fn missing_legacy_path_does_not_create_canonical_files() {
        let tmp = tempfile_dir("regret-paths-missing");
        let repo = tmp.join("repo");
        let home = tmp.join("home");
        fs::create_dir_all(&repo).unwrap();
        let paths = resolve_for_repo_from_env(&repo, env_for_home(&home));

        migrate_legacy(&paths).unwrap();

        assert!(!paths.config_file.exists());
        assert!(!home.join(".cmdrvl/migrations/applied.jsonl").exists());

        fs::remove_dir_all(tmp).ok();
    }

    #[test]
    fn old_only_config_is_copied_and_recorded() {
        let tmp = tempfile_dir("regret-paths-old-only");
        let repo = tmp.join("repo");
        let home = tmp.join("home");
        let legacy = repo.join(".regret/config.toml");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, "[ranking]\ndefault_since = \"14d\"\n").unwrap();
        let paths = resolve_for_repo_from_env(&repo, env_for_home(&home));

        migrate_legacy(&paths).unwrap();

        assert!(fs::read_to_string(&paths.config_file)
            .unwrap()
            .contains("14d"));
        assert!(
            fs::read_to_string(home.join(".cmdrvl/migrations/applied.jsonl"))
                .unwrap()
                .contains("\"path_class\":\"regret_config\"")
        );
        assert!(
            fs::read_to_string(home.join(".cmdrvl/notices/deprecated-paths.jsonl"))
                .unwrap()
                .contains("\"legacy_path_migrated\"")
        );
        assert!(legacy.exists());

        fs::remove_dir_all(tmp).ok();
    }

    #[test]
    fn both_paths_present_prefers_canonical_without_overwrite() {
        let tmp = tempfile_dir("regret-paths-both");
        let repo = tmp.join("repo");
        let home = tmp.join("home");
        let legacy = repo.join(".regret/config.toml");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, "[ranking]\ndefault_since = \"14d\"\n").unwrap();
        let paths = resolve_for_repo_from_env(&repo, env_for_home(&home));
        fs::create_dir_all(paths.config_file.parent().unwrap()).unwrap();
        fs::write(&paths.config_file, "[ranking]\ndefault_since = \"30d\"\n").unwrap();

        migrate_legacy(&paths).unwrap();

        assert!(fs::read_to_string(&paths.config_file)
            .unwrap()
            .contains("30d"));
        assert!(!home.join(".cmdrvl/migrations/applied.jsonl").exists());
        assert!(
            fs::read_to_string(home.join(".cmdrvl/notices/deprecated-paths.jsonl"))
                .unwrap()
                .contains("\"canonical_preferred\"")
        );

        fs::remove_dir_all(tmp).ok();
    }

    #[cfg(unix)]
    #[test]
    fn migrated_file_permissions_are_tightened() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile_dir("regret-paths-permissions");
        let repo = tmp.join("repo");
        let home = tmp.join("home");
        let legacy = repo.join(".regret/config.toml");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, "[scan]\nbootstrap_since = \"60d\"\n").unwrap();
        fs::set_permissions(&legacy, fs::Permissions::from_mode(0o644)).unwrap();
        let paths = resolve_for_repo_from_env(&repo, env_for_home(&home));

        migrate_legacy(&paths).unwrap();

        let mode = fs::metadata(&paths.config_file)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        fs::remove_dir_all(tmp).ok();
    }

    fn tempfile_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{label}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path.canonicalize().unwrap_or(path)
    }
}
