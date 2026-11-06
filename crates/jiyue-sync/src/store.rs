//! opendal-backed S3 and WebDAV `RemoteStore` implementations. Owned by W1-D1.
//!
//! The API is blocking. Each [`OpendalStore`] owns a small tokio runtime (2 worker threads) that
//! is created once in the constructor; every call runs its async operation on that runtime and
//! waits for the result. Call these methods from background threads, never from inside an async
//! task (waiting on a runtime from its own worker would block or panic).

use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opendal::blocking;
use opendal::layers::TimeoutLayer;
use opendal::options::{DeleteOptions, ListOptions, ReadOptions, WriteOptions};
use opendal::services;
use opendal::{Buffer, ErrorKind, HttpTransporter, Metadata, OperationContext, Operator};
use opendal_http_transport_reqwest::ReqwestTransport;
use serde::{Deserialize, Serialize};

use crate::{PutOptions, RemoteEntry, RemoteStore, StoreError};

const REDACTED: &str = "<redacted>";
const NULL_VERSION: &str = "null";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Retries for transient failures. Done in this module, not with opendal's RetryLayer, because
/// that layer reports a failed recursive listing as an empty listing.
const RETRY_MAX_TIMES: usize = 3;
const RETRY_FIRST_DELAY: Duration = Duration::from_millis(200);
const RETRY_MAX_DELAY: Duration = Duration::from_secs(5);
const PROBE_PREFIX: &str = ".jiyue-probe-";
const PROBE_BODY: &[u8] = b"jiyue connection probe\n";
/// How many entries the versioning probe inspects before giving up.
const VERSION_PROBE_LIMIT: usize = 32;

/// Connection settings for an S3 bucket (AWS S3, MinIO or another S3-compatible server).
/// `Debug` redacts the credentials.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Config {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    #[serde(default)]
    pub session_token: Option<String>,
    /// Prefix inside the bucket that holds the vault, e.g. `vaults/my-notes/`.
    #[serde(default)]
    pub root_prefix: String,
    /// Use `endpoint/bucket/key` addressing. Needed by MinIO and most self-hosted servers.
    #[serde(default)]
    pub force_path_style: bool,
}

impl fmt::Debug for S3Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Config")
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field("access_key_id", &REDACTED)
            .field("secret_access_key", &REDACTED)
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| REDACTED),
            )
            .field("root_prefix", &self.root_prefix)
            .field("force_path_style", &self.force_path_style)
            .finish()
    }
}

/// Connection settings for a WebDAV server (Nextcloud, ownCloud, rclone serve webdav, ...).
/// `Debug` redacts the password.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebDavConfig {
    pub endpoint: String,
    pub username: String,
    pub password: String,
    /// Folder on the server that holds the vault, e.g. `jiyue/my-notes/`.
    #[serde(default)]
    pub root: String,
}

impl fmt::Debug for WebDavConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebDavConfig")
            .field("endpoint", &self.endpoint)
            .field("username", &self.username)
            .field("password", &REDACTED)
            .field("root", &self.root)
            .finish()
    }
}

/// Which backend to use, as stored in the app settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "backend", rename_all = "lowercase")]
pub enum StoreConfig {
    S3(S3Config),
    WebDav(WebDavConfig),
}

/// One version of a remote object, as returned by [`OpendalStore::list_version_details`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionEntry {
    pub entry: RemoteEntry,
    /// True for an S3 delete marker. Markers have no etag and no content.
    pub is_delete_marker: bool,
    /// True for the version that a plain `get` returns.
    pub is_latest: bool,
}

/// Result of [`OpendalStore::check_connection`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionReport {
    pub reachable: bool,
    pub can_list: bool,
    pub can_write: bool,
    pub can_delete: bool,
    pub supports_versions: bool,
    pub conditional_put_supported: bool,
    pub message: String,
}

/// A `RemoteStore` on top of an opendal blocking operator.
pub struct OpendalStore {
    label: &'static str,
    // Field order matters: the operator must drop before the runtime it was created on.
    op: blocking::Operator,
    _runtime: tokio::runtime::Runtime,
    /// Credentials that must never appear in messages. Redacted from every error text.
    secrets: Vec<String>,
    /// Cached answer of the versioning probe. `None` means "not known yet".
    versions: Mutex<Option<bool>>,
}

impl fmt::Debug for OpendalStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpendalStore")
            .field("backend", &self.label)
            .finish_non_exhaustive()
    }
}

impl OpendalStore {
    /// Builds a store for the given configuration. Does not contact the server.
    pub fn from_config(config: &StoreConfig) -> Result<Self, StoreError> {
        match config {
            StoreConfig::S3(cfg) => Self::s3(cfg),
            StoreConfig::WebDav(cfg) => Self::webdav(cfg),
        }
    }

    /// Builds an S3 store. Does not contact the server.
    pub fn s3(cfg: &S3Config) -> Result<Self, StoreError> {
        let secrets = secrets_of([
            Some(cfg.access_key_id.as_str()),
            Some(cfg.secret_access_key.as_str()),
            cfg.session_token.as_deref(),
        ]);
        let root = clean_root(&cfg.root_prefix)?;

        let mut builder = services::S3::default()
            .bucket(&cfg.bucket)
            .endpoint(&cfg.endpoint)
            .access_key_id(&cfg.access_key_id)
            .secret_access_key(&cfg.secret_access_key)
            // Never pick up AWS_* environment variables or ~/.aws profiles behind the user's back.
            .disable_config_load();
        if !cfg.region.is_empty() {
            builder = builder.region(&cfg.region);
        }
        if let Some(token) = cfg.session_token.as_deref().filter(|t| !t.is_empty()) {
            builder = builder.session_token(token);
        }
        if !cfg.force_path_style {
            builder = builder.enable_virtual_host_style();
        }
        if !root.is_empty() {
            builder = builder.root(&absolute_root(&root));
        }

        let op = Operator::new(builder).map_err(|e| map_error(e, false, &secrets))?;
        Self::assemble("s3", op, secrets)
    }

    /// Builds a WebDAV store. Does not contact the server.
    pub fn webdav(cfg: &WebDavConfig) -> Result<Self, StoreError> {
        let secrets = secrets_of([Some(cfg.password.as_str())]);
        let root = clean_root(&cfg.root)?;

        let mut builder = services::Webdav::default().endpoint(&cfg.endpoint);
        if !cfg.username.is_empty() {
            builder = builder.username(&cfg.username);
        }
        if !cfg.password.is_empty() {
            builder = builder.password(&cfg.password);
        }
        if !root.is_empty() {
            builder = builder.root(&absolute_root(&root));
        }

        let op = Operator::new(builder).map_err(|e| map_error(e, false, &secrets))?;
        Self::assemble("webdav", op, secrets)
    }

    /// In-memory store for tests. Supports `if_not_exists` writes; has no etags and no versions.
    pub fn memory() -> Result<Self, StoreError> {
        let op =
            Operator::new(services::Memory::default()).map_err(|e| map_error(e, false, &[]))?;
        Self::assemble("memory", op, Vec::new())
    }

    fn assemble(
        label: &'static str,
        op: Operator,
        secrets: Vec<String>,
    ) -> Result<Self, StoreError> {
        let op = op
            .layer(
                TimeoutLayer::new()
                    .with_timeout(REQUEST_TIMEOUT)
                    .with_io_timeout(REQUEST_TIMEOUT),
            )
            .with_context(http_context()?);

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("jiyue-opendal")
            .enable_all()
            .build()
            .map_err(|e| StoreError::Other(format!("cannot start runtime: {e}")))?;
        // The blocking operator captures the handle of the runtime that is current here.
        let op = {
            let _guard = runtime.enter();
            blocking::Operator::new(op).map_err(|e| map_error(e, false, &secrets))?
        };

        Ok(Self {
            label,
            op,
            _runtime: runtime,
            secrets,
            versions: Mutex::new(None),
        })
    }

    /// Lists every version of one object, newest first, including S3 delete markers.
    /// Unlike the trait method this does not check [`RemoteStore::supports_versions`]; it is also
    /// used for cleanup of probe objects.
    pub fn list_version_details(&self, path: &str) -> Result<Vec<VersionEntry>, StoreError> {
        let object = clean_object(path)?;
        let options = ListOptions {
            recursive: true,
            versions: true,
            deleted: true,
            ..Default::default()
        };
        let entries = with_retries(true, || self.op.list_options(&object, options.clone()))
            .map_err(|e| self.map(e, false))?;

        let mut out = Vec::new();
        for entry in &entries {
            let meta = entry.metadata();
            if meta.is_dir() || entry.path().trim_end_matches('/') != object {
                continue;
            }
            out.push(VersionEntry {
                entry: entry_from(&object, meta),
                is_delete_marker: meta.is_deleted(),
                is_latest: meta.is_current() == Some(true),
            });
        }
        Ok(out)
    }

    /// Writes and deletes a probe object, checks conditional creates and reports what works.
    /// Probe objects (including S3 versions and delete markers) are removed before returning.
    pub fn check_connection(&self) -> ConnectionReport {
        let mut notes: Vec<String> = Vec::new();

        let listed = self.list("");
        let can_list = listed.is_ok();
        let reachable = !matches!(listed, Err(StoreError::Network(_)));
        if let Err(err) = &listed {
            notes.push(format!("list failed: {err}"));
        }
        if !reachable {
            return ConnectionReport {
                reachable,
                can_list,
                can_write: false,
                can_delete: false,
                supports_versions: false,
                conditional_put_supported: false,
                message: format!("cannot reach the server: {}", notes.join("; ")),
            };
        }

        let token = random_token();
        let probe = format!("{PROBE_PREFIX}{token}");
        let cond_probe = format!("{PROBE_PREFIX}{token}-cond");
        let mut created: Vec<String> = Vec::new();

        // Plain write: also tells us whether the backend returns version ids.
        let written = self.put(&probe, PROBE_BODY, &PutOptions::default());
        let can_write = written.is_ok();
        let mut supports_versions = false;
        match &written {
            Ok(entry) => {
                created.push(probe.clone());
                supports_versions = entry.version_id.is_some();
                *self.versions_cache() = Some(supports_versions);
            }
            Err(err) => notes.push(format!("write failed: {err}")),
        }
        if !can_write {
            supports_versions = self.supports_versions();
        }

        // Conditional create: the second create of the same key must be refused.
        let mut conditional_put_supported = false;
        if can_write {
            let create = PutOptions {
                if_match: None,
                if_none_match_star: true,
            };
            match self.put(&cond_probe, PROBE_BODY, &create) {
                Ok(_) => {
                    created.push(cond_probe.clone());
                    match self.put(&cond_probe, PROBE_BODY, &create) {
                        Err(StoreError::PreconditionFailed) => conditional_put_supported = true,
                        Ok(_) => notes.push(
                            "conditional create does not refuse existing objects".to_string(),
                        ),
                        Err(err) => notes.push(format!("second conditional create: {err}")),
                    }
                }
                Err(err) => notes.push(format!("conditional create unsupported: {err}")),
            }
        }

        // Delete everything we created, including versions and delete markers.
        let mut can_delete = true;
        for path in &created {
            if let Err(err) = self.delete_probe(path) {
                can_delete = false;
                notes.push(format!("cleanup of {path} failed: {err}"));
            }
        }

        let mut parts = vec![
            format!("list {}", ok_word(can_list)),
            format!("write {}", ok_word(can_write)),
            format!("delete {}", ok_word(can_delete)),
            format!(
                "versioning {}",
                if supports_versions { "on" } else { "off" }
            ),
            format!(
                "conditional create {}",
                if conditional_put_supported {
                    "ok"
                } else {
                    "not available"
                }
            ),
        ];
        parts.extend(notes);

        ConnectionReport {
            reachable,
            can_list,
            can_write,
            can_delete,
            supports_versions,
            conditional_put_supported,
            message: parts.join("; "),
        }
    }

    fn versions_cache(&self) -> MutexGuard<'_, Option<bool>> {
        self.versions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Converts an opendal error, redacting secrets from its text.
    fn map(&self, err: opendal::Error, conditional: bool) -> StoreError {
        map_error(err, conditional, &self.secrets)
    }

    /// Plain delete that treats a missing object as success.
    fn delete_plain(&self, object: &str) -> Result<(), StoreError> {
        match with_retries(true, || self.op.delete(object)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            Err(err) => Err(self.map(err, false)),
        }
    }

    /// Removes a probe object: plain delete, then every version and delete marker it left.
    fn delete_probe(&self, object: &str) -> Result<(), StoreError> {
        self.delete_plain(object)?;
        for version in self.list_version_details(object)? {
            let Some(id) = version.entry.version_id else {
                continue;
            };
            let options = DeleteOptions {
                version: Some(id),
                ..Default::default()
            };
            with_retries(true, || self.op.delete_options(object, options.clone()))
                .map_err(|e| self.map(e, false))?;
        }
        match self.stat(object)? {
            None => Ok(()),
            Some(_) => Err(StoreError::Other("probe object still present".into())),
        }
    }

    /// Asks the server whether versions exist. Returns `None` when the listing is empty or
    /// fails, because then nothing is known.
    fn probe_versions(&self) -> Option<bool> {
        let options = ListOptions {
            recursive: true,
            versions: true,
            deleted: true,
            limit: Some(VERSION_PROBE_LIMIT),
            ..Default::default()
        };
        let lister = self.op.lister_options("", options).ok()?;
        let mut seen = 0usize;
        for item in lister.take(VERSION_PROBE_LIMIT) {
            let entry = item.ok()?;
            seen += 1;
            let meta = entry.metadata();
            if let Some(id) = meta.version()
                && id != NULL_VERSION
            {
                return Some(true);
            }
        }
        if seen == 0 { None } else { Some(false) }
    }
}

impl RemoteStore for OpendalStore {
    fn list(&self, prefix: &str) -> Result<Vec<RemoteEntry>, StoreError> {
        let dir = clean_dir(prefix)?;
        let options = ListOptions {
            recursive: true,
            ..Default::default()
        };
        let entries = with_retries(true, || self.op.list_options(&dir, options.clone()))
            .map_err(|e| self.map(e, false))?;

        let own = dir.trim_end_matches('/');
        let mut out = Vec::with_capacity(entries.len());
        for entry in &entries {
            let path = entry.path().trim_end_matches('/');
            if path.is_empty() || path == own {
                continue;
            }
            out.push(entry_from(path, entry.metadata()));
        }
        Ok(out)
    }

    fn stat(&self, path: &str) -> Result<Option<RemoteEntry>, StoreError> {
        let object = clean_object(path)?;
        match with_retries(true, || self.op.stat(&object)) {
            Ok(meta) => Ok(Some(entry_from(&object, &meta))),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(self.map(err, false)),
        }
    }

    fn get(&self, path: &str, version_id: Option<&str>) -> Result<Vec<u8>, StoreError> {
        let object = clean_object(path)?;
        let version = version_id.filter(|v| !v.is_empty());
        let result = with_retries(true, || match version {
            Some(version) => {
                let options = ReadOptions {
                    version: Some(version.to_string()),
                    ..Default::default()
                };
                self.op.read_options(&object, options)
            }
            None => self.op.read(&object),
        });
        result
            .map(|buffer| buffer.to_vec())
            .map_err(|e| self.map(e, false))
    }

    fn put(&self, path: &str, data: &[u8], opts: &PutOptions) -> Result<RemoteEntry, StoreError> {
        let object = clean_object(path)?;
        let conditional = opts.if_match.is_some() || opts.if_none_match_star;
        let options = WriteOptions {
            if_match: opts.if_match.clone(),
            if_not_exists: opts.if_none_match_star,
            ..Default::default()
        };
        let body = Buffer::from(data.to_vec());
        // Unconditional writes are idempotent and may be retried. A conditional write that
        // reached the server but lost its response must not be replayed: the replay would fail
        // with a false PreconditionFailed.
        let meta = with_retries(!conditional, || {
            self.op
                .write_options(&object, body.clone(), options.clone())
        })
        .map_err(|e| self.map(e, conditional))?;

        let mut entry = if meta.etag().is_some() {
            entry_from(&object, &meta)
        } else {
            // The backend did not return an etag: read the object back for the real metadata.
            let stat =
                with_retries(true, || self.op.stat(&object)).map_err(|e| self.map(e, false))?;
            entry_from(&object, &stat)
        };
        entry.size = data.len() as u64;
        Ok(entry)
    }

    fn delete(&self, path: &str) -> Result<(), StoreError> {
        let object = clean_object(path)?;
        self.delete_plain(&object)
    }

    fn list_versions(&self, path: &str) -> Result<Vec<RemoteEntry>, StoreError> {
        if !self.supports_versions() {
            return Ok(Vec::new());
        }
        Ok(self
            .list_version_details(path)?
            .into_iter()
            .map(|version| version.entry)
            .collect())
    }

    fn supports_versions(&self) -> bool {
        let cached = *self.versions_cache();
        if let Some(known) = cached {
            return known;
        }
        match self.probe_versions() {
            Some(found) => {
                *self.versions_cache() = Some(found);
                found
            }
            None => false,
        }
    }
}

/// Runs `attempt`, retrying transient failures with exponential backoff when `retry` is true.
fn with_retries<T>(
    retry: bool,
    mut attempt: impl FnMut() -> opendal::Result<T>,
) -> opendal::Result<T> {
    let mut delay = RETRY_FIRST_DELAY;
    let mut retries = 0usize;
    loop {
        match attempt() {
            Err(err) if retry && retries < RETRY_MAX_TIMES && is_transient(&err) => {
                retries += 1;
                std::thread::sleep(delay);
                delay = (delay * 2).min(RETRY_MAX_DELAY);
            }
            other => return other,
        }
    }
}

/// Failures worth retrying: opendal's temporary errors and connection-level failures.
fn is_transient(err: &opendal::Error) -> bool {
    err.is_temporary() || is_network_failure(err)
}

/// Maps an opendal error to a [`StoreError`]. `conditional` is true when the request carried a
/// condition, which lets a 409 conflict mean "precondition failed".
fn map_error(err: opendal::Error, conditional: bool, secrets: &[String]) -> StoreError {
    let kind = err.kind();
    let network = is_transient(&err);
    let text = redact(&err.to_string(), secrets);
    match kind {
        ErrorKind::NotFound => StoreError::NotFound,
        ErrorKind::ConditionNotMatch => StoreError::PreconditionFailed,
        ErrorKind::Conflict if conditional => StoreError::PreconditionFailed,
        ErrorKind::PermissionDenied => StoreError::Unauthorized,
        _ if network => StoreError::Network(text),
        _ => StoreError::Other(text),
    }
}

/// opendal marks some connection failures as persistent, so look at the underlying reqwest
/// error: refused connections, DNS failures and timeouts count as network failures.
fn is_network_failure(err: &opendal::Error) -> bool {
    use std::error::Error as _;
    let mut current: Option<&(dyn std::error::Error + 'static)> = err.source();
    while let Some(cause) = current {
        if let Some(http) = cause.downcast_ref::<reqwest::Error>() {
            return http.is_connect() || http.is_timeout();
        }
        current = cause.source();
    }
    false
}

fn http_context() -> Result<OperationContext, StoreError> {
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        // Per read, not total: large files keep going as long as bytes keep arriving.
        .read_timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| StoreError::Other(format!("cannot build HTTP client: {e}")))?;
    Ok(OperationContext::new()
        .with_http_transport(HttpTransporter::new(ReqwestTransport::new(client))))
}

fn secrets_of<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> Vec<String> {
    values
        .into_iter()
        .flatten()
        .filter(|s| s.len() >= 3)
        .map(str::to_owned)
        .collect()
}

fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        out = out.replace(secret.as_str(), REDACTED);
    }
    out
}

fn ok_word(ok: bool) -> &'static str {
    if ok { "ok" } else { "failed" }
}

fn random_token() -> String {
    let mut hasher = RandomState::new().build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    hasher.write(&nanos.to_le_bytes());
    hasher.write_u32(std::process::id());
    format!("{:016x}", hasher.finish())
}

fn invalid_path() -> StoreError {
    StoreError::Other("invalid path".into())
}

/// Cleans a relative remote path: `\` becomes `/`, empty and `.` segments are dropped, and
/// absolute paths, Windows drive paths, `..` segments and NUL bytes are rejected. Unicode names
/// are kept as they are. The result has no leading or trailing slash (empty for the root).
fn clean_path(raw: &str) -> Result<String, StoreError> {
    let unified = raw.replace('\\', "/");
    if unified.starts_with('/') || has_drive_prefix(&unified) || unified.contains('\0') {
        return Err(invalid_path());
    }
    let mut parts: Vec<&str> = Vec::new();
    for segment in unified.split('/') {
        match segment {
            "" | "." => {}
            ".." => return Err(invalid_path()),
            other => parts.push(other),
        }
    }
    Ok(parts.join("/"))
}

/// True for `C:` or `C:/...`, but not for names such as `a:b.md`.
fn has_drive_prefix(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || b[2] == b'/')
}

/// Cleans an object path. Rejects the empty path, which names no object.
fn clean_object(raw: &str) -> Result<String, StoreError> {
    let cleaned = clean_path(raw)?;
    if cleaned.is_empty() {
        return Err(invalid_path());
    }
    Ok(cleaned)
}

/// Cleans a folder prefix. Returns `""` for the root, otherwise `a/b/` with a trailing slash.
fn clean_dir(raw: &str) -> Result<String, StoreError> {
    let cleaned = clean_path(raw)?;
    if cleaned.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("{cleaned}/"))
    }
}

/// Cleans a configured root. A leading slash is tolerated here (`/vaults/x/` means
/// `vaults/x/` inside the bucket or server); `..` and drive paths are still rejected.
fn clean_root(raw: &str) -> Result<String, StoreError> {
    clean_dir(raw.trim_start_matches(['/', '\\']))
}

/// opendal wants roots with a leading and a trailing slash.
fn absolute_root(dir: &str) -> String {
    format!("/{dir}")
}

fn entry_from(path: &str, meta: &Metadata) -> RemoteEntry {
    RemoteEntry {
        path: path.trim_end_matches('/').to_string(),
        size: meta.content_length(),
        etag: meta.etag().map(str::to_owned),
        modified_ms: meta
            .last_modified()
            .map(|ts| ts.into_inner().as_millisecond()),
        version_id: meta
            .version()
            .filter(|v| *v != NULL_VERSION)
            .map(str::to_owned),
        is_dir: meta.is_dir(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> OpendalStore {
        OpendalStore::memory().expect("memory store")
    }

    fn put_plain(store: &OpendalStore, path: &str, body: &[u8]) -> RemoteEntry {
        store
            .put(path, body, &PutOptions::default())
            .expect("plain put")
    }

    #[test]
    fn clean_path_normalises_separators_and_keeps_unicode() {
        assert_eq!(clean_path("a\\b\\c.md").unwrap(), "a/b/c.md");
        assert_eq!(clean_path("a//./b/").unwrap(), "a/b");
        assert_eq!(
            clean_path("笔记/中文 名 (1).md").unwrap(),
            "笔记/中文 名 (1).md"
        );
        assert_eq!(clean_path("a:b.md").unwrap(), "a:b.md");
        assert_eq!(clean_path("").unwrap(), "");
    }

    #[test]
    fn clean_path_rejects_escapes_and_absolute_paths() {
        for bad in [
            "../x",
            "a/../../x",
            "a\\..\\x",
            "/etc/passwd",
            "\\\\server\\share",
            "C:/x",
            "C:\\x",
            "a\0b",
        ] {
            assert_eq!(
                clean_path(bad),
                Err(invalid_path()),
                "{bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn object_and_dir_cleaning() {
        assert_eq!(clean_object("notes/a.md").unwrap(), "notes/a.md");
        assert_eq!(clean_object(""), Err(invalid_path()));
        assert_eq!(clean_object("./"), Err(invalid_path()));
        assert_eq!(clean_dir("").unwrap(), "");
        assert_eq!(clean_dir("vaults\\my notes").unwrap(), "vaults/my notes/");
        assert_eq!(absolute_root("vaults/x/"), "/vaults/x/");
        assert_eq!(clean_root("/vaults/x").unwrap(), "vaults/x/");
        assert_eq!(clean_root("\\vaults\\x\\").unwrap(), "vaults/x/");
        assert_eq!(clean_root("/../x"), Err(invalid_path()));
    }

    #[test]
    fn transient_failures_are_retried_only_when_allowed() {
        let transient = || opendal::Error::new(ErrorKind::Unexpected, "busy").set_temporary();

        let mut calls = 0;
        let recovered = with_retries(true, || {
            calls += 1;
            if calls < 3 {
                Err(transient())
            } else {
                Ok(calls)
            }
        });
        assert_eq!(recovered.unwrap(), 3);

        let mut calls = 0;
        let refused: opendal::Result<()> = with_retries(false, || {
            calls += 1;
            Err(transient())
        });
        assert!(refused.is_err());
        assert_eq!(calls, 1, "conditional writes must not be replayed");

        let mut calls = 0;
        let exhausted: opendal::Result<()> = with_retries(true, || {
            calls += 1;
            Err(transient())
        });
        assert!(exhausted.is_err());
        assert_eq!(calls, RETRY_MAX_TIMES + 1);

        let mut calls = 0;
        let permanent: opendal::Result<()> = with_retries(true, || {
            calls += 1;
            Err(opendal::Error::new(ErrorKind::NotFound, "gone"))
        });
        assert!(permanent.is_err());
        assert_eq!(calls, 1, "permanent errors are not retried");
    }

    #[test]
    fn error_mapping_covers_each_category() {
        let plain = |kind| opendal::Error::new(kind, "boom");
        assert_eq!(
            map_error(plain(ErrorKind::NotFound), false, &[]),
            StoreError::NotFound
        );
        assert_eq!(
            map_error(plain(ErrorKind::ConditionNotMatch), true, &[]),
            StoreError::PreconditionFailed
        );
        assert_eq!(
            map_error(plain(ErrorKind::Conflict), true, &[]),
            StoreError::PreconditionFailed
        );
        assert!(matches!(
            map_error(plain(ErrorKind::Conflict), false, &[]),
            StoreError::Other(_)
        ));
        assert_eq!(
            map_error(plain(ErrorKind::PermissionDenied), false, &[]),
            StoreError::Unauthorized
        );
        let transient =
            opendal::Error::new(ErrorKind::Unexpected, "connect timeout").set_temporary();
        assert!(
            matches!(map_error(transient, false, &[]), StoreError::Network(m) if m.contains("connect timeout"))
        );
        assert!(matches!(
            map_error(plain(ErrorKind::Unexpected), false, &[]),
            StoreError::Other(_)
        ));
        assert!(matches!(
            map_error(plain(ErrorKind::Unsupported), false, &[]),
            StoreError::Other(_)
        ));
    }

    #[test]
    fn error_text_never_contains_secrets() {
        let secret = "hunter2-secret";
        let err = opendal::Error::new(ErrorKind::Unexpected, format!("auth with {secret} failed"));
        let mapped = map_error(err, false, &[secret.to_string()]);
        let text = match mapped {
            StoreError::Other(text) | StoreError::Network(text) => text,
            other => panic!("unexpected {other:?}"),
        };
        assert!(!text.contains(secret));
        assert!(text.contains(REDACTED));
    }

    #[test]
    fn debug_output_redacts_credentials() {
        let s3 = S3Config {
            endpoint: "http://127.0.0.1:9000".into(),
            region: "us-east-1".into(),
            bucket: "notes".into(),
            access_key_id: "AKIAVISIBLEKEYID".into(),
            secret_access_key: "very-secret-value".into(),
            session_token: Some("token-value-123".into()),
            root_prefix: "vaults/a/".into(),
            force_path_style: true,
        };
        let text = format!("{s3:?}");
        assert!(!text.contains("very-secret-value"));
        assert!(!text.contains("token-value-123"));
        assert!(!text.contains("AKIAVISIBLEKEYID"));
        assert!(text.contains("127.0.0.1"));

        let dav = WebDavConfig {
            endpoint: "https://dav.example".into(),
            username: "me".into(),
            password: "pw-very-secret".into(),
            root: String::new(),
        };
        let text = format!("{:?}", StoreConfig::WebDav(dav));
        assert!(!text.contains("pw-very-secret"));
        assert!(text.contains("me"));
    }

    #[test]
    fn store_config_round_trips_through_json() {
        let config = StoreConfig::S3(S3Config {
            endpoint: "http://127.0.0.1:9000".into(),
            region: "us-east-1".into(),
            bucket: "notes".into(),
            access_key_id: "k".into(),
            secret_access_key: "s".into(),
            session_token: None,
            root_prefix: "vaults/x/".into(),
            force_path_style: true,
        });
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"backend\":\"s3\""));
        let back: StoreConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, config);

        let dav: StoreConfig = serde_json::from_str(
            r#"{"backend":"webdav","endpoint":"https://d","username":"u","password":"p"}"#,
        )
        .unwrap();
        assert!(matches!(dav, StoreConfig::WebDav(ref c) if c.root.is_empty()));
    }

    #[test]
    fn constructors_build_without_network() {
        let s3 = S3Config {
            endpoint: "http://127.0.0.1:9".into(),
            region: "us-east-1".into(),
            bucket: "notes".into(),
            access_key_id: "k".into(),
            secret_access_key: "secret-key".into(),
            session_token: Some("tok".into()),
            root_prefix: "/vaults/x/".into(),
            force_path_style: false,
        };
        if let Err(err) = OpendalStore::s3(&s3) {
            panic!("s3 constructor failed: {err}");
        }
        let dav = WebDavConfig {
            endpoint: "http://127.0.0.1:9".into(),
            username: "u".into(),
            password: "p".into(),
            root: "jiyue/中文 库/".into(),
        };
        assert!(OpendalStore::from_config(&StoreConfig::WebDav(dav)).is_ok());
        let bad_root = WebDavConfig {
            endpoint: "http://127.0.0.1:9".into(),
            username: String::new(),
            password: String::new(),
            root: "../escape".into(),
        };
        assert_eq!(OpendalStore::webdav(&bad_root).unwrap_err(), invalid_path());
    }

    #[test]
    fn memory_put_get_stat_list_delete() {
        let store = memory();
        let written = put_plain(&store, "notes/a.md", b"hello");
        assert_eq!(written.path, "notes/a.md");
        assert_eq!(written.size, 5);
        assert!(!written.is_dir);

        assert_eq!(store.get("notes/a.md", None).unwrap(), b"hello");
        let stat = store.stat("notes/a.md").unwrap().expect("exists");
        assert_eq!(stat.size, 5);

        put_plain(&store, "notes/sub/b.md", b"bb");
        put_plain(&store, "top.md", b"t");
        let mut paths: Vec<String> = store
            .list("")
            .unwrap()
            .into_iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.path)
            .collect();
        paths.sort();
        assert_eq!(paths, vec!["notes/a.md", "notes/sub/b.md", "top.md"]);

        let under_notes: Vec<String> = store
            .list("notes")
            .unwrap()
            .into_iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.path)
            .collect();
        assert_eq!(under_notes.len(), 2);
        assert!(under_notes.iter().all(|p| p.starts_with("notes/")));
        assert!(under_notes.iter().all(|p| !p.starts_with('/')));

        store.delete("notes/a.md").unwrap();
        assert_eq!(store.stat("notes/a.md").unwrap(), None);
        assert_eq!(store.get("notes/a.md", None), Err(StoreError::NotFound));
        // Deleting a missing object is not an error.
        store.delete("notes/a.md").unwrap();
    }

    #[test]
    fn memory_directories_are_flagged() {
        let store = memory();
        put_plain(&store, "dir/inner.md", b"x");
        // Recursive listings report files; directories are only flagged when the backend has them.
        let entries = store.list("").unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.path == "dir/inner.md" && !e.is_dir)
        );
        assert!(entries.iter().all(|e| !e.path.ends_with('/')));
        assert!(entries.iter().all(|e| e.path != "dir" || e.is_dir));
    }

    #[test]
    fn memory_path_edge_cases_and_unicode() {
        let store = memory();
        let name = "笔记/中文 名 (1)+#%.md";
        put_plain(&store, name, "内容".as_bytes());
        assert_eq!(store.get(name, None).unwrap(), "内容".as_bytes());
        // Backslashes and redundant separators address the same object.
        assert_eq!(
            store.get("笔记\\中文 名 (1)+#%.md", None).unwrap(),
            "内容".as_bytes()
        );
        assert_eq!(
            store.get("笔记//中文 名 (1)+#%.md", None).unwrap(),
            "内容".as_bytes()
        );

        assert_eq!(store.get("../secret", None), Err(invalid_path()));
        assert_eq!(store.stat("/abs.md"), Err(invalid_path()));
        assert_eq!(
            store.put("a/../b", b"x", &PutOptions::default()),
            Err(invalid_path())
        );
        assert_eq!(store.delete("C:\\x"), Err(invalid_path()));
        assert_eq!(store.list("../up"), Err(invalid_path()));
    }

    #[test]
    fn memory_if_none_match_star_refuses_second_create() {
        let store = memory();
        let create = PutOptions {
            if_match: None,
            if_none_match_star: true,
        };
        store.put("lock.md", b"one", &create).unwrap();
        assert_eq!(
            store.put("lock.md", b"two", &create),
            Err(StoreError::PreconditionFailed)
        );
        assert_eq!(store.get("lock.md", None).unwrap(), b"one");
    }

    #[test]
    fn memory_if_match_is_reported_as_unsupported() {
        // The memory service has no etags, so an etag condition cannot be honoured. The store
        // reports that instead of silently overwriting. Real etag semantics are covered by the
        // ignored S3 integration test.
        let store = memory();
        put_plain(&store, "a.md", b"x");
        let conditional = PutOptions {
            if_match: Some("\"etag\"".into()),
            if_none_match_star: false,
        };
        assert!(matches!(
            store.put("a.md", b"y", &conditional),
            Err(StoreError::Other(_))
        ));
        assert_eq!(store.get("a.md", None).unwrap(), b"x");
    }

    #[test]
    fn memory_has_no_versions() {
        let store = memory();
        put_plain(&store, "a.md", b"1");
        put_plain(&store, "a.md", b"2");
        assert!(!store.supports_versions());
        assert_eq!(store.list_versions("a.md").unwrap(), Vec::new());
        // Asking for a version the backend does not know is an error, not silent latest.
        assert!(store.get("a.md", Some("v1")).is_err());
    }

    #[test]
    fn memory_check_connection_reports_and_cleans_up() {
        let store = memory();
        put_plain(&store, "keep.md", b"k");
        let report = store.check_connection();
        assert!(report.reachable);
        assert!(report.can_list);
        assert!(report.can_write);
        assert!(report.can_delete);
        assert!(!report.supports_versions);
        assert!(report.conditional_put_supported, "{}", report.message);
        assert!(report.message.contains("versioning off"));

        let names: Vec<String> = store
            .list("")
            .unwrap()
            .into_iter()
            .map(|e| e.path)
            .collect();
        assert_eq!(
            names,
            vec!["keep.md".to_string()],
            "probe objects must be removed"
        );
    }

    #[test]
    fn unreachable_endpoint_is_reported_without_panicking() {
        let dav = WebDavConfig {
            endpoint: "http://127.0.0.1:1".into(),
            username: String::new(),
            password: String::new(),
            root: String::new(),
        };
        let store = OpendalStore::webdav(&dav).unwrap();
        // Recursive listing used to come back as Ok([]) here (opendal RetryLayer). It must now fail
        // with a network error, and quickly: connection refused is immediate, retries add ~1.4 s.
        let started = std::time::Instant::now();
        let listed = store.list("");
        assert!(
            matches!(listed, Err(StoreError::Network(_))),
            "listing a dead endpoint must return Network, got {listed:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(30));
        let report = store.check_connection();
        assert!(!report.reachable, "{}", report.message);
        assert!(!report.can_write);
        assert!(!report.supports_versions);
        assert!(!report.conditional_put_supported);
        assert!(!report.message.is_empty());
    }

    /// Versioned S3 test scope: a random prefix in the configured bucket. Dropping it removes every
    /// version and delete marker below the prefix, so a failing test does not leave data behind.
    struct S3TestScope {
        store: OpendalStore,
    }

    impl S3TestScope {
        fn open() -> Self {
            let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
            let (Some(endpoint), Some(bucket), Some(key), Some(secret)) = (
                env("JIYUE_TEST_S3_ENDPOINT"),
                env("JIYUE_TEST_S3_BUCKET"),
                env("JIYUE_TEST_S3_KEY"),
                env("JIYUE_TEST_S3_SECRET"),
            ) else {
                panic!("set JIYUE_TEST_S3_ENDPOINT, _BUCKET, _KEY and _SECRET to run this test");
            };
            let region = env("JIYUE_TEST_S3_REGION").unwrap_or_else(|| "us-east-1".into());
            let store = OpendalStore::s3(&S3Config {
                endpoint,
                region,
                bucket,
                access_key_id: key,
                secret_access_key: secret,
                session_token: None,
                root_prefix: format!("jiyue-it-{}/", random_token()),
                force_path_style: true,
            })
            .expect("store");
            Self { store }
        }

        /// Every version and delete marker below the scope prefix.
        fn all_versions(&self) -> Result<Vec<VersionEntry>, String> {
            let options = ListOptions {
                recursive: true,
                versions: true,
                deleted: true,
                ..Default::default()
            };
            let entries = self
                .store
                .op
                .list_options("", options)
                .map_err(|e| e.to_string())?;
            Ok(entries
                .iter()
                .filter(|e| !e.metadata().is_dir())
                .map(|e| VersionEntry {
                    entry: entry_from(e.path(), e.metadata()),
                    is_delete_marker: e.metadata().is_deleted(),
                    is_latest: e.metadata().is_current() == Some(true),
                })
                .collect())
        }

        /// Removes everything below the scope prefix; returns how many versions were deleted.
        fn purge(&self) -> Result<usize, String> {
            let versions = self.all_versions()?;
            for version in &versions {
                let Some(id) = version.entry.version_id.clone() else {
                    continue;
                };
                self.store
                    .op
                    .delete_options(
                        &version.entry.path,
                        DeleteOptions {
                            version: Some(id),
                            ..Default::default()
                        },
                    )
                    .map_err(|e| e.to_string())?;
            }
            Ok(versions.len())
        }
    }

    impl Drop for S3TestScope {
        fn drop(&mut self) {
            if let Err(err) = self.purge() {
                eprintln!("S3 test cleanup failed: {err}");
            }
        }
    }

    /// Probe objects from check_connection must leave nothing behind, not even versions.
    #[test]
    #[ignore = "needs JIYUE_TEST_S3_* environment variables (see README.md)"]
    fn s3_check_connection_reports_versions_and_conditional_writes() {
        let scope = S3TestScope::open();
        let report = scope.store.check_connection();
        assert!(
            report.reachable && report.can_list && report.can_write && report.can_delete,
            "{}",
            report.message
        );
        assert!(report.supports_versions, "{}", report.message);
        assert!(report.conditional_put_supported, "{}", report.message);
        let leftovers = scope.all_versions().unwrap();
        assert!(
            leftovers.is_empty(),
            "probe objects left behind: {leftovers:?}"
        );
    }

    /// Three writes give three versions; old versions stay readable; delete adds a marker.
    #[test]
    #[ignore = "needs JIYUE_TEST_S3_* environment variables (see README.md)"]
    fn s3_versions_and_delete_markers() {
        let scope = S3TestScope::open();
        let s = &scope.store;
        let first = put_plain(s, "history.md", b"version one");
        put_plain(s, "history.md", b"version two");
        let third = put_plain(s, "history.md", b"version three");
        let first_id = first.version_id.clone().expect("S3 returns a version id");
        let third_id = third.version_id.clone().expect("S3 returns a version id");
        assert_ne!(first_id, third_id);

        let versions = s.list_versions("history.md").unwrap();
        assert_eq!(versions.len(), 3, "expected 3 versions: {versions:?}");
        assert!(
            versions
                .iter()
                .any(|v| v.version_id.as_deref() == Some(first_id.as_str()))
        );
        assert!(versions.iter().all(|v| v.etag.is_some()));

        assert_eq!(
            s.get("history.md", Some(&first_id)).unwrap(),
            b"version one"
        );
        assert_eq!(s.get("history.md", None).unwrap(), b"version three");
        assert_eq!(
            s.get("history.md", Some(&third_id)).unwrap(),
            b"version three"
        );

        s.delete("history.md").unwrap();
        assert_eq!(s.stat("history.md").unwrap(), None);
        assert_eq!(s.get("history.md", None), Err(StoreError::NotFound));

        let after = s.list_version_details("history.md").unwrap();
        assert_eq!(after.len(), 4, "history must survive delete: {after:?}");
        let markers: Vec<_> = after.iter().filter(|v| v.is_delete_marker).collect();
        assert_eq!(markers.len(), 1);
        assert!(markers[0].is_latest);
        assert!(
            markers[0].entry.etag.is_none(),
            "delete markers carry no etag"
        );
        assert_eq!(
            s.get("history.md", Some(&first_id)).unwrap(),
            b"version one"
        );

        // The deleted key is not in a normal listing, but versions still show the path.
        assert!(s.list("").unwrap().iter().all(|e| e.path != "history.md"));
    }

    /// If-None-Match: * refuses an existing key; If-Match only writes over the current etag.
    #[test]
    #[ignore = "needs JIYUE_TEST_S3_* environment variables (see README.md)"]
    fn s3_conditional_put_semantics() {
        let scope = S3TestScope::open();
        let s = &scope.store;
        let create = PutOptions {
            if_none_match_star: true,
            ..Default::default()
        };
        let first = s.put("lock.md", b"owner-a", &create).unwrap();
        assert_eq!(
            s.put("lock.md", b"owner-b", &create),
            Err(StoreError::PreconditionFailed)
        );

        let wrong = PutOptions {
            if_match: Some("\"00000000000000000000000000000000\"".into()),
            ..Default::default()
        };
        assert_eq!(
            s.put("lock.md", b"x", &wrong),
            Err(StoreError::PreconditionFailed)
        );
        assert_eq!(s.get("lock.md", None).unwrap(), b"owner-a");

        let etag = first.etag.clone().expect("S3 returns an etag");
        let matching = PutOptions {
            if_match: Some(etag.clone()),
            ..Default::default()
        };
        let second = s.put("lock.md", b"owner-a2", &matching).unwrap();
        assert_ne!(second.etag, Some(etag.clone()));
        assert_eq!(s.get("lock.md", None).unwrap(), b"owner-a2");

        // The first etag is now stale.
        assert_eq!(
            s.put("lock.md", b"stale", &matching),
            Err(StoreError::PreconditionFailed)
        );
        assert_eq!(s.get("lock.md", None).unwrap(), b"owner-a2");
    }

    /// Non-ASCII and punctuation keys round-trip, and a recursive list under a prefix returns
    /// only that subtree, with relative paths and no trailing slashes.
    #[test]
    #[ignore = "needs JIYUE_TEST_S3_* environment variables (see README.md)"]
    fn s3_unicode_keys_and_recursive_listing() {
        let scope = S3TestScope::open();
        let s = &scope.store;
        let names = [
            "笔记/中文 名 (1)+#%.md",
            "笔记/子目录/深 层.md",
            "top 文件.md",
        ];
        for name in names {
            put_plain(s, name, name.as_bytes());
        }
        for name in names {
            assert_eq!(s.get(name, None).unwrap(), name.as_bytes(), "{name}");
            assert!(s.stat(name).unwrap().is_some(), "{name}");
        }

        let mut all: Vec<String> = s
            .list("")
            .unwrap()
            .into_iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.path)
            .collect();
        all.sort();
        let mut expected: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        expected.sort();
        assert_eq!(all, expected);
        assert!(all.iter().all(|p| !p.starts_with('/') && !p.ends_with('/')));

        let mut under: Vec<String> = s
            .list("笔记")
            .unwrap()
            .into_iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.path)
            .collect();
        under.sort();
        assert_eq!(
            under,
            vec![
                "笔记/中文 名 (1)+#%.md".to_string(),
                "笔记/子目录/深 层.md".to_string(),
            ],
            "recursive listing under a folder must include nested files"
        );
    }

    /// A 5 MiB object round-trips byte for byte, with the right size from stat and list.
    #[test]
    #[ignore = "needs JIYUE_TEST_S3_* environment variables (see README.md)"]
    fn s3_five_megabyte_object_round_trip() {
        let scope = S3TestScope::open();
        let s = &scope.store;
        let body: Vec<u8> = (0..5 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let written = put_plain(s, "big/blob.bin", &body);
        assert_eq!(written.size, body.len() as u64);
        assert_eq!(
            s.stat("big/blob.bin").unwrap().expect("exists").size,
            body.len() as u64
        );
        let listed = s.list("big").unwrap();
        assert!(
            listed
                .iter()
                .any(|e| e.path == "big/blob.bin" && e.size == body.len() as u64)
        );
        let back = s.get("big/blob.bin", None).unwrap();
        assert_eq!(back.len(), body.len());
        assert!(back == body, "object content differs after round trip");
    }

    /// WebDAV against a real server. Not run by default and skipped when the environment is not
    /// set. Real WebDAV is unverified until the user runs it with:
    /// `JIYUE_TEST_DAV_ENDPOINT`, `JIYUE_TEST_DAV_USER`, `JIYUE_TEST_DAV_PASSWORD`.
    /// Uses a random folder; the folder itself is left empty on the server.
    #[test]
    #[ignore = "needs JIYUE_TEST_DAV_* environment variables and a WebDAV server"]
    fn webdav_integration_round_trip_with_unicode_names() {
        let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let Some(endpoint) = env("JIYUE_TEST_DAV_ENDPOINT") else {
            eprintln!(
                "SKIPPED webdav_integration: set JIYUE_TEST_DAV_ENDPOINT (and _USER, _PASSWORD)"
            );
            return;
        };
        let store = OpendalStore::webdav(&WebDavConfig {
            endpoint,
            username: env("JIYUE_TEST_DAV_USER").unwrap_or_default(),
            password: env("JIYUE_TEST_DAV_PASSWORD").unwrap_or_default(),
            root: format!("jiyue-it-{}/", random_token()),
        })
        .expect("store");

        let report = store.check_connection();
        assert!(
            report.reachable && report.can_write && report.can_delete,
            "{}",
            report.message
        );
        assert!(!report.supports_versions);

        let name = "笔记 库/中文 名 (1).md";
        put_plain(&store, name, "内容 with spaces".as_bytes());
        assert_eq!(
            store.get(name, None).unwrap(),
            "内容 with spaces".as_bytes()
        );
        assert!(store.stat(name).unwrap().is_some());
        store.delete(name).unwrap();
        assert_eq!(store.stat(name).unwrap(), None);
    }
}
