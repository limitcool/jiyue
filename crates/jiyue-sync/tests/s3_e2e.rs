//! End-to-end sync scenarios against a real, versioned S3-compatible bucket.
//!
//! Every test is `#[ignore]` and needs these environment variables (the bucket must have
//! versioning enabled, because the version and history checks depend on it):
//!
//! - `JIYUE_TEST_S3_ENDPOINT`, `JIYUE_TEST_S3_BUCKET`, `JIYUE_TEST_S3_KEY`, `JIYUE_TEST_S3_SECRET`
//! - `JIYUE_TEST_S3_REGION` (optional, default `us-east-1`)
//!
//! Run them with (from the repository root):
//! `cargo test -p jiyue-sync --test s3_e2e -- --ignored --test-threads=1 --nocapture`
//!
//! Each test works under its own random prefix `jiyue-e2e-<id>/` in the bucket and removes every
//! version and delete marker below it when it finishes (also after a failure). Two local vault
//! folders per test are created under the system temp directory and removed the same way.
//!
//! Secrets are read from the environment only and never printed.
//!
//! The non-ignored tests in this file use the in-memory store and need no server.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use jiyue_sync::state::{load_state, now_ms};
use jiyue_sync::store::{OpendalStore, S3Config};
use jiyue_sync::{
    ConflictKind, PutOptions, RemoteEntry, RemoteStore, StoreError, SyncOptions, SyncReport,
    run_sync,
};

// ---------------------------------------------------------------------------------------------
// Configuration and cleanup
// ---------------------------------------------------------------------------------------------

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A short id that is unique across tests and runs.
fn unique_id() -> String {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!(
        "{:x}{:04x}{:02x}",
        now_ms() as u64,
        std::process::id() % 0xffff,
        n
    )
}

/// Connection settings for a fresh random prefix in the test bucket.
fn s3_config() -> S3Config {
    let (Some(endpoint), Some(bucket), Some(key), Some(secret)) = (
        env("JIYUE_TEST_S3_ENDPOINT"),
        env("JIYUE_TEST_S3_BUCKET"),
        env("JIYUE_TEST_S3_KEY"),
        env("JIYUE_TEST_S3_SECRET"),
    ) else {
        panic!("set JIYUE_TEST_S3_ENDPOINT, _BUCKET, _KEY and _SECRET (see the file header)");
    };
    S3Config {
        endpoint,
        region: env("JIYUE_TEST_S3_REGION").unwrap_or_else(|| "us-east-1".into()),
        bucket,
        access_key_id: key,
        secret_access_key: secret,
        session_token: None,
        root_prefix: format!("jiyue-e2e-{}/", unique_id()),
        force_path_style: true,
    }
}

/// Removes every version and delete marker under the prefix of `cfg`. Uses opendal directly, so
/// it does not depend on the code under test.
fn purge(cfg: &S3Config) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let builder = opendal::services::S3::default()
            .bucket(&cfg.bucket)
            .endpoint(&cfg.endpoint)
            .region(&cfg.region)
            .access_key_id(&cfg.access_key_id)
            .secret_access_key(&cfg.secret_access_key)
            .root(&format!("/{}", cfg.root_prefix))
            .disable_config_load();
        let Ok(op) = opendal::Operator::new(builder) else {
            eprintln!("cleanup: cannot build the operator");
            return;
        };
        // Same HTTP transport as the store under test (opendal needs one set explicitly here).
        let client = reqwest::Client::builder().build().expect("http client");
        let context =
            opendal::OperationContext::new().with_http_transport(opendal::HttpTransporter::new(
                opendal_http_transport_reqwest::ReqwestTransport::new(client),
            ));
        let op = op.with_context(context);
        let options = opendal::options::ListOptions {
            recursive: true,
            versions: true,
            deleted: true,
            ..Default::default()
        };
        match op.list_options("", options).await {
            Ok(entries) => {
                for entry in entries.iter().filter(|e| !e.metadata().is_dir()) {
                    let version = entry.metadata().version().map(str::to_owned);
                    let result = match version {
                        Some(id) if id != "null" => {
                            op.delete_options(
                                entry.path(),
                                opendal::options::DeleteOptions {
                                    version: Some(id),
                                    ..Default::default()
                                },
                            )
                            .await
                        }
                        _ => op.delete(entry.path()).await,
                    };
                    if let Err(e) = result {
                        eprintln!("cleanup: {} failed: {}", entry.path(), e.kind());
                    }
                }
            }
            Err(e) => eprintln!("cleanup: listing failed: {}", e.kind()),
        }
    });
}

/// Two vault folders plus the remote prefix. Dropping it cleans up both.
struct Fixture {
    cfg: S3Config,
    base: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!("jiyue-e2e-{}", unique_id()));
        fs::create_dir_all(base.join("A")).expect("vault A");
        fs::create_dir_all(base.join("B")).expect("vault B");
        Self {
            cfg: s3_config(),
            base,
        }
    }

    fn a(&self) -> PathBuf {
        self.base.join("A")
    }

    fn b(&self) -> PathBuf {
        self.base.join("B")
    }

    fn store(&self) -> OpendalStore {
        OpendalStore::s3(&self.cfg).expect("store")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
        purge(&self.cfg);
    }
}

// ---------------------------------------------------------------------------------------------
// Probe: a RemoteStore wrapper that injects failures and records successful writes
// ---------------------------------------------------------------------------------------------

/// Wraps a store. Operations are numbered from 0. Operation `n >= fail_from` fails with a network
/// error (a lost connection), operation `panic_at` panics (the process is killed mid-run).
struct Probe<'a> {
    inner: &'a dyn RemoteStore,
    ops: AtomicUsize,
    fail_from: AtomicUsize,
    panic_at: AtomicUsize,
    fail_list: AtomicBool,
    /// Puts whose path contains this text fail with a server error.
    fail_put_containing: Mutex<Option<String>>,
    /// Paths of successful puts, in order.
    puts: Mutex<Vec<String>>,
}

impl<'a> Probe<'a> {
    fn new(inner: &'a dyn RemoteStore) -> Self {
        Self {
            inner,
            ops: AtomicUsize::new(0),
            fail_from: AtomicUsize::new(usize::MAX),
            panic_at: AtomicUsize::new(usize::MAX),
            fail_list: AtomicBool::new(false),
            fail_put_containing: Mutex::new(None),
            puts: Mutex::new(Vec::new()),
        }
    }

    fn tick(&self, what: &str) -> Result<(), StoreError> {
        let n = self.ops.fetch_add(1, Ordering::SeqCst);
        if n == self.panic_at.load(Ordering::SeqCst) {
            panic!("injected kill at operation {n} ({what})");
        }
        if n >= self.fail_from.load(Ordering::SeqCst) {
            return Err(StoreError::Network("injected: connection lost".into()));
        }
        Ok(())
    }

    fn put_paths(&self) -> Vec<String> {
        self.puts.lock().expect("puts").clone()
    }
}

impl RemoteStore for Probe<'_> {
    fn list(&self, prefix: &str) -> Result<Vec<RemoteEntry>, StoreError> {
        self.tick("list")?;
        if self.fail_list.load(Ordering::SeqCst) {
            return Err(StoreError::Network("injected: listing failed".into()));
        }
        self.inner.list(prefix)
    }

    fn stat(&self, path: &str) -> Result<Option<RemoteEntry>, StoreError> {
        self.tick("stat")?;
        self.inner.stat(path)
    }

    fn get(&self, path: &str, version_id: Option<&str>) -> Result<Vec<u8>, StoreError> {
        self.tick("get")?;
        self.inner.get(path, version_id)
    }

    fn put(&self, path: &str, data: &[u8], opts: &PutOptions) -> Result<RemoteEntry, StoreError> {
        self.tick("put")?;
        let fail = self.fail_put_containing.lock().expect("fail rule").clone();
        if let Some(text) = fail
            && path.contains(&text)
        {
            return Err(StoreError::Other("injected: server error on write".into()));
        }
        let stored = self.inner.put(path, data, opts)?;
        self.puts.lock().expect("puts").push(path.to_owned());
        Ok(stored)
    }

    fn delete(&self, path: &str) -> Result<(), StoreError> {
        self.tick("delete")?;
        self.inner.delete(path)
    }

    fn list_versions(&self, path: &str) -> Result<Vec<RemoteEntry>, StoreError> {
        self.inner.list_versions(path)
    }

    fn supports_versions(&self) -> bool {
        self.inner.supports_versions()
    }
}

// ---------------------------------------------------------------------------------------------
// Vault and remote helpers
// ---------------------------------------------------------------------------------------------

/// Every file in a vault, keyed by vault-relative path. `.jiyue` and `.trash` are excluded.
fn vault_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).expect("read dir") {
            let entry = entry.expect("entry");
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".jiyue" || name == ".trash" {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .expect("under root")
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.insert(rel, fs::read(&path).expect("read file"));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// Every file on the remote (through the store API), keyed by path.
fn remote_tree(store: &dyn RemoteStore) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for entry in store.list("").expect("list remote") {
        if entry.is_dir {
            continue;
        }
        let data = store.get(&entry.path, None).expect("get remote");
        out.insert(entry.path, data);
    }
    out
}

fn write(root: &Path, rel: &str, data: &[u8]) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(&path, data).expect("write");
}

fn read_text(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Deterministic pseudo-random bytes (so that binary content is not text by accident).
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
        | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// One sync pass. The state is loaded from disk each time, as the app does.
fn sync_once(root: &Path, store: &dyn RemoteStore, device: &str) -> SyncReport {
    let mut state = load_state(root).expect("load state");
    let mut opts = SyncOptions::new(device);
    run_sync(root, store, &mut state, &mut opts)
}

/// One sync pass that must finish without fatal or per-file errors.
fn sync_clean(root: &Path, store: &dyn RemoteStore, device: &str) -> SyncReport {
    let report = sync_once(root, store, device);
    assert!(report.fatal.is_none(), "{device}: fatal {:?}", report.fatal);
    assert!(
        report.errors.is_empty(),
        "{device}: errors {:?}",
        report.errors
    );
    report
}

fn assert_idle(report: &SyncReport, device: &str) {
    assert!(
        report.uploaded.is_empty()
            && report.downloaded.is_empty()
            && report.merged.is_empty()
            && report.conflicts.is_empty()
            && report.deleted_local.is_empty()
            && report.deleted_remote.is_empty(),
        "{device}: expected a no-op run, got {report:?}"
    );
}

/// Files of the trash folder under the vault, as `name -> bytes` (names keep their stamp).
fn trash_files(root: &Path) -> Vec<Vec<u8>> {
    let dir = root.join(".trash").join("sync");
    let Ok(read) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    read.flatten()
        .filter(|e| e.path().is_file())
        .map(|e| fs::read(e.path()).expect("trash file"))
        .collect()
}

/// Text used for the shared note in the merge scenarios: 20 numbered lines.
fn numbered_note() -> String {
    (1..=20).map(|i| format!("line {i:02}\n")).collect()
}

/// A standard two-device setup: A creates a varied vault and syncs, B syncs and receives it.
fn two_devices(fx: &Fixture, store: &dyn RemoteStore) {
    let a = fx.a();
    write(
        &a,
        "笔记/中文 名 (1)+#%.md",
        "# 中文\n内容 with spaces\n".as_bytes(),
    );
    write(&a, "daily/2026-11-06.md", b"# daily\n- item\n");
    write(&a, "深 层/子目录/a b.md", "嵌套 nested\n".as_bytes());
    write(&a, "empty.md", b"");
    write(&a, "attachments/图 片 1.png", &noise(300 * 1024, 7));
    write(&a, "top.txt", b"plain text\n");
    write(&a, "merge.md", numbered_note().as_bytes());
    write(&a, "delete-me.md", b"will be deleted\n");
    write(&a, "delete-me2.md", b"will be deleted too\n");
    let up = sync_clean(&a, store, "DEVA");
    assert_eq!(
        up.uploaded.len(),
        9,
        "first sync uploads every file: {up:?}"
    );

    let down = sync_clean(&fx.b(), store, "DEVB");
    assert_eq!(down.downloaded.len(), 9, "B receives every file: {down:?}");
    assert_eq!(
        vault_tree(&fx.a()),
        vault_tree(&fx.b()),
        "byte-identical after first sync"
    );
}

// ---------------------------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------------------------

/// A creates notes with CJK names, spaces, nested folders, a binary attachment and an empty file.
/// B receives them byte for byte, and a second run on each side is a no-op.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header)"]
fn s3_e2e_first_sync_is_byte_identical() {
    let fx = Fixture::new();
    let store = fx.store();
    two_devices(&fx, &store);
    assert_idle(&sync_clean(&fx.a(), &store, "DEVA"), "A");
    assert_idle(&sync_clean(&fx.b(), &store, "DEVB"), "B");
    assert_eq!(remote_tree(&store), vault_tree(&fx.a()), "remote equals A");
    assert_eq!(
        vault_tree(&fx.b())
            .get("attachments/图 片 1.png")
            .map(Vec::len),
        Some(300 * 1024)
    );
    assert_eq!(vault_tree(&fx.b()).get("empty.md").map(Vec::len), Some(0));
}

/// B edits note 1 and A edits note 2 (different files). Both sides converge.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header)"]
fn s3_e2e_independent_edits_converge() {
    let fx = Fixture::new();
    let store = fx.store();
    two_devices(&fx, &store);

    write(
        &fx.b(),
        "daily/2026-11-06.md",
        b"# daily\n- item\n- from B\n",
    );
    sync_clean(&fx.b(), &store, "DEVB");
    write(&fx.a(), "top.txt", b"plain text\nfrom A\n");
    let a = sync_clean(&fx.a(), &store, "DEVA");
    assert!(
        a.downloaded.contains(&"daily/2026-11-06.md".to_owned()),
        "{a:?}"
    );
    assert!(a.uploaded.contains(&"top.txt".to_owned()), "{a:?}");
    sync_clean(&fx.b(), &store, "DEVB");
    sync_clean(&fx.a(), &store, "DEVA");

    assert_eq!(vault_tree(&fx.a()), vault_tree(&fx.b()));
    assert_eq!(remote_tree(&store), vault_tree(&fx.a()));
    assert!(read_text(&fx.a(), "daily/2026-11-06.md").contains("from B"));
    assert!(read_text(&fx.b(), "top.txt").contains("from A"));
    assert_idle(&sync_clean(&fx.a(), &store, "DEVA"), "A");
    assert_idle(&sync_clean(&fx.b(), &store, "DEVB"), "B");
}

/// A and B edit different parts of the same note: the merge is clean and keeps both edits.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header)"]
fn s3_e2e_disjoint_edits_merge_cleanly() {
    let fx = Fixture::new();
    let store = fx.store();
    two_devices(&fx, &store);

    write(
        &fx.a(),
        "merge.md",
        numbered_note()
            .replace("line 02", "line 02 by A")
            .as_bytes(),
    );
    sync_clean(&fx.a(), &store, "DEVA");
    write(
        &fx.b(),
        "merge.md",
        numbered_note()
            .replace("line 18", "line 18 by B")
            .as_bytes(),
    );
    let b = sync_clean(&fx.b(), &store, "DEVB");
    assert!(b.merged.contains(&"merge.md".to_owned()), "{b:?}");
    assert!(b.conflicts.is_empty(), "{b:?}");
    sync_clean(&fx.a(), &store, "DEVA");

    for root in [fx.a(), fx.b()] {
        let text = read_text(&root, "merge.md");
        assert!(text.contains("line 02 by A"), "{text}");
        assert!(text.contains("line 18 by B"), "{text}");
        assert!(!text.contains("<<<<<<<"), "no markers after a clean merge");
    }
    assert_eq!(vault_tree(&fx.a()), vault_tree(&fx.b()));
    assert_eq!(remote_tree(&store), vault_tree(&fx.a()));
}

/// A and B edit the same line: B gets a conflict. Nothing is lost on either side, and after one
/// more sync on A both sides hold the same file set.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header)"]
fn s3_e2e_same_line_edit_makes_conflict_copy_and_converges() {
    let fx = Fixture::new();
    let store = fx.store();
    two_devices(&fx, &store);

    let a_bin = noise(4096, 11);
    write(&fx.a(), "attachments/图 片 1.png", &a_bin);
    sync_clean(&fx.a(), &store, "DEVA");

    write(
        &fx.a(),
        "merge.md",
        numbered_note()
            .replace("line 05", "line 05 A-version")
            .as_bytes(),
    );
    sync_clean(&fx.a(), &store, "DEVA");
    write(
        &fx.b(),
        "merge.md",
        numbered_note()
            .replace("line 05", "line 05 B-version")
            .as_bytes(),
    );
    let b_bin = noise(4096, 12);
    write(&fx.b(), "attachments/图 片 1.png", &b_bin);

    let b = sync_clean(&fx.b(), &store, "DEVB");
    let kinds: Vec<(String, ConflictKind)> = b
        .conflicts
        .iter()
        .map(|c| (c.path.clone(), c.kind))
        .collect();
    assert!(
        kinds.contains(&("merge.md".to_owned(), ConflictKind::Markers)),
        "text conflict expected: {b:?}"
    );
    assert!(
        kinds.contains(&("attachments/图 片 1.png".to_owned(), ConflictKind::BothKept)),
        "binary conflict expected: {b:?}"
    );
    let copies: Vec<String> = b.conflicts.iter().map(|c| c.copy_path.clone()).collect();
    for copy in &copies {
        assert!(fx.b().join(copy).exists(), "copy {copy} on B");
    }

    // Nothing lost on B: both text versions are present, and both binaries are kept.
    let merged = read_text(&fx.b(), "merge.md");
    assert!(
        merged.contains("A-version") && merged.contains("B-version"),
        "{merged}"
    );
    let all_b: Vec<Vec<u8>> = vault_tree(&fx.b()).into_values().collect();
    assert!(all_b.contains(&b_bin), "B's binary kept");
    assert!(all_b.contains(&a_bin), "A's binary kept as a copy");

    sync_clean(&fx.a(), &store, "DEVA");
    sync_clean(&fx.b(), &store, "DEVB");
    assert_eq!(
        vault_tree(&fx.a()),
        vault_tree(&fx.b()),
        "same file set after another sync"
    );
    assert_eq!(remote_tree(&store), vault_tree(&fx.a()));
    assert_idle(&sync_clean(&fx.a(), &store, "DEVA"), "A");
    assert_idle(&sync_clean(&fx.b(), &store, "DEVB"), "B");
}

/// A deletes a note that B edits. Whichever side syncs last, the edit survives.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header)"]
fn s3_e2e_delete_versus_edit_keeps_the_edit() {
    let fx = Fixture::new();
    let store = fx.store();
    two_devices(&fx, &store);

    // Order 1: A deletes locally, B edits and uploads, A syncs.
    fs::remove_file(fx.a().join("delete-me.md")).expect("delete on A");
    write(&fx.b(), "delete-me.md", b"B kept editing this\n");
    sync_clean(&fx.b(), &store, "DEVB");
    let a = sync_clean(&fx.a(), &store, "DEVA");
    assert!(a.downloaded.contains(&"delete-me.md".to_owned()), "{a:?}");
    assert_eq!(read_text(&fx.a(), "delete-me.md"), "B kept editing this\n");

    // Order 2: A deletes and syncs (remote delete), then B edits and syncs.
    fs::remove_file(fx.a().join("delete-me2.md")).expect("delete on A");
    sync_clean(&fx.a(), &store, "DEVA");
    write(&fx.b(), "delete-me2.md", b"B edited after A deleted\n");
    let b = sync_clean(&fx.b(), &store, "DEVB");
    assert!(b.uploaded.contains(&"delete-me2.md".to_owned()), "{b:?}");
    sync_clean(&fx.a(), &store, "DEVA");
    assert_eq!(
        read_text(&fx.a(), "delete-me2.md"),
        "B edited after A deleted\n"
    );
    assert_eq!(vault_tree(&fx.a()), vault_tree(&fx.b()));
}

/// A renames a note. B ends with the new name only (no duplicate), the old name goes to trash.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header)"]
fn s3_e2e_rename_leaves_one_file() {
    let fx = Fixture::new();
    let store = fx.store();
    two_devices(&fx, &store);

    fs::rename(
        fx.a().join("daily/2026-11-06.md"),
        fx.a().join("daily/renamed.md"),
    )
    .expect("rename on A");
    sync_clean(&fx.a(), &store, "DEVA");
    sync_clean(&fx.b(), &store, "DEVB");

    let tree = vault_tree(&fx.b());
    assert!(tree.contains_key("daily/renamed.md"));
    assert!(
        !tree.contains_key("daily/2026-11-06.md"),
        "old name must be gone on B"
    );
    let copies = tree
        .values()
        .filter(|v| v.as_slice() == b"# daily\n- item\n")
        .count();
    assert_eq!(copies, 1, "exactly one copy of the renamed note");
    assert_eq!(tree, vault_tree(&fx.a()));
    assert_eq!(remote_tree(&store), tree);
    assert!(
        !trash_files(&fx.b()).is_empty(),
        "old name moved to trash, not erased"
    );
}

/// B deletes a note. A's copy goes to trash (never erased), and the remote no longer has it.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header)"]
fn s3_e2e_remote_delete_moves_local_copy_to_trash() {
    let fx = Fixture::new();
    let store = fx.store();
    two_devices(&fx, &store);

    fs::remove_file(fx.b().join("top.txt")).expect("delete on B");
    let b = sync_clean(&fx.b(), &store, "DEVB");
    assert!(b.deleted_remote.contains(&"top.txt".to_owned()), "{b:?}");
    let a = sync_clean(&fx.a(), &store, "DEVA");
    assert!(a.deleted_local.contains(&"top.txt".to_owned()), "{a:?}");
    assert!(!fx.a().join("top.txt").exists());
    assert!(trash_files(&fx.a()).contains(&b"plain text\n".to_vec()));
    assert!(!remote_tree(&store).contains_key("top.txt"));
    assert_eq!(vault_tree(&fx.a()), vault_tree(&fx.b()));
}

/// A 20 MB attachment round-trips, and the timings are printed.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header)"]
fn s3_e2e_twenty_megabyte_attachment() {
    let fx = Fixture::new();
    let store = fx.store();
    let body = noise(20 * 1024 * 1024, 99);
    write(&fx.a(), "attachments/big 大文件.bin", &body);

    let t = Instant::now();
    sync_clean(&fx.a(), &store, "DEVA");
    eprintln!("[timing] 20 MB first upload: {:?}", t.elapsed());

    let t = Instant::now();
    assert_idle(&sync_clean(&fx.a(), &store, "DEVA"), "A");
    eprintln!("[timing] 20 MB no-change sync on A: {:?}", t.elapsed());

    let t = Instant::now();
    sync_clean(&fx.b(), &store, "DEVB");
    eprintln!("[timing] 20 MB download on B: {:?}", t.elapsed());
    let got = fs::read(fx.b().join("attachments/big 大文件.bin")).expect("B has it");
    assert!(got == body, "20 MB content differs after the round trip");
}

/// 500 small notes: first sync, no-change sync, one-edit sync, and the first sync on B.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header)"]
fn s3_e2e_five_hundred_notes_timings() {
    let fx = Fixture::new();
    let store = fx.store();
    for i in 0..500 {
        let body = format!("# note {i}\n{}\n", "正文 text ".repeat(100 + i % 50));
        write(
            &fx.a(),
            &format!("notes/{:02}/n{i:03} 笔记.md", i % 10),
            body.as_bytes(),
        );
    }

    let t = Instant::now();
    let up = sync_clean(&fx.a(), &store, "DEVA");
    eprintln!("[timing] 500 notes first sync on A: {:?}", t.elapsed());
    assert_eq!(up.uploaded.len(), 500);

    let t = Instant::now();
    assert_idle(&sync_clean(&fx.a(), &store, "DEVA"), "A");
    eprintln!("[timing] 500 notes no-change sync on A: {:?}", t.elapsed());

    write(&fx.a(), "notes/03/n003 笔记.md", b"one edit\n");
    let t = Instant::now();
    let one = sync_clean(&fx.a(), &store, "DEVA");
    eprintln!("[timing] 500 notes one-edit sync on A: {:?}", t.elapsed());
    assert_eq!(one.uploaded, vec!["notes/03/n003 笔记.md".to_owned()]);

    let t = Instant::now();
    let down = sync_clean(&fx.b(), &store, "DEVB");
    eprintln!("[timing] 500 notes first sync on B: {:?}", t.elapsed());
    assert_eq!(down.downloaded.len(), 500);
    assert_eq!(vault_tree(&fx.a()), vault_tree(&fx.b()));
}

/// A lost connection in the middle of an upload run. The run reports per-file errors, and the next
/// run finishes the job: every file is uploaded once, none twice.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header)"]
fn s3_e2e_interrupted_run_resumes_without_duplicates() {
    let fx = Fixture::new();
    let real = fx.store();
    for i in 0..120 {
        write(
            &fx.a(),
            &format!("batch/n{i:03}.md"),
            format!("note {i}\n").as_bytes(),
        );
    }

    let probe = Probe::new(&real);
    probe.fail_from.store(60, Ordering::SeqCst);
    let first = sync_once(&fx.a(), &probe, "DEVA");
    let first_puts = probe.put_paths();
    assert!(
        !first.errors.is_empty(),
        "the lost connection must be reported"
    );
    assert!(
        first
            .fatal
            .as_deref()
            .is_some_and(|f| f.contains("network error")),
        "the run must stop with a network error: {first:?}"
    );
    // Three failures in a row stop the run: the list (op 0), 59 good puts, then ops 60..=62 fail.
    assert_eq!(
        first.errors.len(),
        3,
        "the run stops after three network failures in a row"
    );
    assert_eq!(
        probe.ops.load(Ordering::SeqCst),
        63,
        "no attempts after the stop"
    );
    assert!(first_puts.len() < 120, "some files must have failed");
    eprintln!(
        "[resume] run 1: {} uploaded, {} per-file errors",
        first_puts.len(),
        first.errors.len()
    );

    let second_probe = Probe::new(&real);
    let second = sync_clean(&fx.a(), &second_probe, "DEVA");
    let second_puts = second_probe.put_paths();
    assert_eq!(
        first_puts.len() + second_puts.len(),
        120,
        "every file uploaded once"
    );
    let mut all: Vec<String> = first_puts
        .iter()
        .chain(second_puts.iter())
        .cloned()
        .collect();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 120, "no file uploaded twice");
    assert_eq!(second.uploaded.len(), second_puts.len());
    assert_eq!(remote_tree(&real), vault_tree(&fx.a()));
}

/// The process dies inside a run (a panic in the store). Notes uploaded after the last checkpoint
/// are not recorded in the state; the next run recognises them as identical and does not upload
/// them again. Nothing is lost.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header)"]
fn s3_e2e_killed_run_resumes_without_reupload() {
    let fx = Fixture::new();
    let real = fx.store();
    for i in 0..120 {
        write(
            &fx.a(),
            &format!("kill/n{i:03}.md"),
            format!("note {i}\n").as_bytes(),
        );
    }

    let probe = Probe::new(&real);
    probe.panic_at.store(45, Ordering::SeqCst);
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sync_once(&fx.a(), &probe, "DEVA")
    }));
    assert!(crashed.is_err(), "the injected kill must abort the run");
    let killed_puts = probe.put_paths();
    eprintln!("[kill] {} uploads before the kill", killed_puts.len());

    let second_probe = Probe::new(&real);
    sync_clean(&fx.a(), &second_probe, "DEVA");
    let second_puts = second_probe.put_paths();
    let mut all: Vec<String> = killed_puts
        .iter()
        .chain(second_puts.iter())
        .cloned()
        .collect();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 120, "every note on the remote");
    assert_eq!(
        killed_puts.len() + second_puts.len(),
        120,
        "no note uploaded twice, even those after the last checkpoint"
    );
    assert_eq!(remote_tree(&real), vault_tree(&fx.a()));
}

/// Version history on a versioned bucket: three edits give three versions; restoring an old version
/// (get by version id, write it back, sync) makes it the latest; a remote delete leaves a delete
/// marker and the old content stays recoverable.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header)"]
fn s3_e2e_version_history_restore_and_delete_marker() {
    let fx = Fixture::new();
    let store = fx.store();

    write(&fx.a(), "history.md", b"version one\n");
    sync_clean(&fx.a(), &store, "DEVA");
    // Versioning is probed from existing objects, so the check comes after the first upload.
    assert!(
        store.supports_versions(),
        "the test bucket must have versioning enabled"
    );
    write(&fx.a(), "history.md", b"version two\n");
    sync_clean(&fx.a(), &store, "DEVA");
    write(&fx.a(), "history.md", b"version three\n");
    sync_clean(&fx.a(), &store, "DEVA");

    let versions = store.list_versions("history.md").expect("versions");
    assert_eq!(versions.len(), 3, "{versions:?}");
    // Same length for all three texts, so pick the version by its content, not its size.
    let mut first = None;
    for v in &versions {
        let Some(id) = v.version_id.clone() else {
            continue;
        };
        if store.get("history.md", Some(&id)).expect("read version") == b"version one\n" {
            first = Some(id);
        }
    }
    let first = first.expect("version one is in the history");
    let old = store.get("history.md", Some(&first)).expect("old version");
    assert_eq!(old, b"version one\n");

    // Restore (what the history dialog does): write the old text, then sync.
    write(&fx.a(), "history.md", &old);
    sync_clean(&fx.a(), &store, "DEVA");
    assert_eq!(
        store.get("history.md", None).expect("latest"),
        b"version one\n"
    );
    assert_eq!(
        store.list_versions("history.md").expect("versions").len(),
        4
    );

    // Remote delete: a marker, and the history keeps the content.
    fs::remove_file(fx.a().join("history.md")).expect("delete locally");
    sync_clean(&fx.a(), &store, "DEVA");
    assert_eq!(store.stat("history.md").expect("stat"), None);
    let rows = store
        .list_version_details("history.md")
        .expect("version details");
    assert!(
        rows.iter().any(|v| v.is_delete_marker && v.is_latest),
        "{rows:?}"
    );
    let recoverable = rows
        .iter()
        .filter(|v| !v.is_delete_marker)
        .filter_map(|v| v.entry.version_id.clone())
        .any(|id| {
            store
                .get("history.md", Some(&id))
                .is_ok_and(|d| d == b"version three\n")
        });
    assert!(
        recoverable,
        "the deleted note must be recoverable from history"
    );
}

/// Wrong credentials: a clear unauthorized error, quickly, and no secret in the text.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header)"]
fn s3_e2e_wrong_credentials_are_reported() {
    let fx = Fixture::new();
    let mut cfg = fx.cfg.clone();
    cfg.secret_access_key = "wrong-secret-0123456789".into();
    let store = OpendalStore::s3(&cfg).expect("store");
    write(&fx.a(), "x.md", b"x\n");

    let t = Instant::now();
    let report = sync_once(&fx.a(), &store, "DEVA");
    let took = t.elapsed();
    eprintln!("[timing] wrong credentials failed after {took:?}");
    let fatal = report.fatal.clone().unwrap_or_default();
    assert!(
        fatal.contains("unauthorized"),
        "unexpected message: {fatal}"
    );
    assert!(!fatal.contains("wrong-secret"), "secret leaked: {fatal}");
    assert!(took < Duration::from_secs(20), "must not hang: {took:?}");
    assert!(fx.a().join("x.md").exists(), "nothing touched");
    assert!(report.uploaded.is_empty());
}

/// A closed port: a clear network error within a few seconds.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header); uses a closed local port"]
fn s3_e2e_closed_port_fails_fast_with_network_error() {
    let fx = Fixture::new();
    let mut cfg = fx.cfg.clone();
    cfg.endpoint = "http://127.0.0.1:1".into();
    let store = OpendalStore::s3(&cfg).expect("store");
    write(&fx.a(), "x.md", b"x\n");

    let t = Instant::now();
    let report = sync_once(&fx.a(), &store, "DEVA");
    let took = t.elapsed();
    eprintln!("[timing] closed port failed after {took:?}");
    let fatal = report.fatal.clone().unwrap_or_default();
    assert!(
        fatal.contains("network error"),
        "unexpected message: {fatal}"
    );
    assert!(took < Duration::from_secs(30), "{took:?}");
}

/// An address that drops packets: the connect timeout (10 s per attempt) must end the run.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header); takes about a minute"]
fn s3_e2e_blackholed_endpoint_times_out() {
    let fx = Fixture::new();
    let mut cfg = fx.cfg.clone();
    cfg.endpoint = "http://192.0.2.1:9100".into();
    let store = OpendalStore::s3(&cfg).expect("store");
    write(&fx.a(), "x.md", b"x\n");

    let t = Instant::now();
    let report = sync_once(&fx.a(), &store, "DEVA");
    let took = t.elapsed();
    eprintln!("[timing] blackholed endpoint failed after {took:?}");
    assert!(report.fatal.is_some(), "must fail");
    assert!(took < Duration::from_secs(120), "{took:?}");
    assert!(report.uploaded.is_empty());
}

/// A failing listing aborts the run. It must never be read as "the remote is empty", which would
/// move every local note to trash.
#[test]
#[ignore = "needs JIYUE_TEST_S3_* (see the file header)"]
fn s3_e2e_failing_listing_changes_nothing() {
    let fx = Fixture::new();
    let store = fx.store();
    two_devices(&fx, &store);
    let before = vault_tree(&fx.a());
    let remote_before = remote_tree(&store);

    let probe = Probe::new(&store);
    probe.fail_list.store(true, Ordering::SeqCst);
    let report = sync_once(&fx.a(), &probe, "DEVA");
    assert!(report.fatal.is_some(), "a failed listing is fatal");
    assert!(report.deleted_local.is_empty() && report.deleted_remote.is_empty());
    assert!(report.uploaded.is_empty() && report.downloaded.is_empty());
    assert_eq!(vault_tree(&fx.a()), before, "no local file changed");
    assert!(trash_files(&fx.a()).is_empty(), "nothing moved to trash");
    assert_eq!(remote_tree(&store), remote_before, "remote untouched");
}

// ---------------------------------------------------------------------------------------------
// Server-free regression tests (in-process store with etags, no version history)
// ---------------------------------------------------------------------------------------------

/// A flat object store with etags and conditional writes (`If-Match`, `If-None-Match: *`), the
/// behaviour the sync engine relies on. Not versioned, like a bucket without versioning. It stands
/// in for a real server in tests that must run without one.
#[derive(Default)]
struct EtagStore {
    objects: Mutex<BTreeMap<String, (Vec<u8>, String)>>,
    seq: AtomicUsize,
}

impl RemoteStore for EtagStore {
    fn list(&self, prefix: &str) -> Result<Vec<RemoteEntry>, StoreError> {
        let objects = self.objects.lock().expect("objects");
        Ok(objects
            .iter()
            .filter(|(path, _)| path.starts_with(prefix))
            .map(|(path, (data, etag))| RemoteEntry {
                path: path.clone(),
                size: data.len() as u64,
                etag: Some(etag.clone()),
                modified_ms: None,
                version_id: None,
                is_dir: false,
            })
            .collect())
    }

    fn stat(&self, path: &str) -> Result<Option<RemoteEntry>, StoreError> {
        Ok(self.list(path)?.into_iter().find(|e| e.path == path))
    }

    fn get(&self, path: &str, version_id: Option<&str>) -> Result<Vec<u8>, StoreError> {
        if version_id.is_some() {
            return Err(StoreError::Other("no versions in this store".into()));
        }
        match self.objects.lock().expect("objects").get(path) {
            Some((data, _)) => Ok(data.clone()),
            None => Err(StoreError::NotFound),
        }
    }

    fn put(&self, path: &str, data: &[u8], opts: &PutOptions) -> Result<RemoteEntry, StoreError> {
        let mut objects = self.objects.lock().expect("objects");
        let current = objects.get(path).map(|(_, etag)| etag.clone());
        if opts.if_none_match_star && current.is_some() {
            return Err(StoreError::PreconditionFailed);
        }
        if let Some(expected) = &opts.if_match
            && current.as_deref() != Some(expected.as_str())
        {
            return Err(StoreError::PreconditionFailed);
        }
        let etag = format!("\"e{}\"", self.seq.fetch_add(1, Ordering::SeqCst));
        objects.insert(path.to_owned(), (data.to_vec(), etag.clone()));
        Ok(RemoteEntry {
            path: path.to_owned(),
            size: data.len() as u64,
            etag: Some(etag),
            modified_ms: None,
            version_id: None,
            is_dir: false,
        })
    }

    fn delete(&self, path: &str) -> Result<(), StoreError> {
        self.objects.lock().expect("objects").remove(path);
        Ok(())
    }

    fn list_versions(&self, _path: &str) -> Result<Vec<RemoteEntry>, StoreError> {
        Ok(Vec::new())
    }

    fn supports_versions(&self) -> bool {
        false
    }
}

/// A binary file changed on both devices. The conflict copy cannot be written (server error on
/// the copy name). The remote version must not be lost: it has to survive in the remote, or in the
/// vault, after the failed run. A later run then makes the conflict copy.
#[test]
fn conflict_copy_write_failure_never_loses_the_remote_version() {
    let server = EtagStore::default();
    let base = std::env::temp_dir().join(format!("jiyue-e2e-{}", unique_id()));
    let (a, b) = (base.join("A"), base.join("B"));
    fs::create_dir_all(&a).expect("A");
    fs::create_dir_all(&b).expect("B");

    let v_base = noise(2048, 1);
    let v_a = noise(2048, 2);
    let v_b = noise(2048, 3);
    write(&a, "pic.png", &v_base);
    sync_clean(&a, &server, "DEVA");
    sync_clean(&b, &server, "DEVB");
    write(&a, "pic.png", &v_a);
    assert_no_conflict(&sync_clean(&a, &server, "DEVA"));
    write(&b, "pic.png", &v_b);

    let probe = Probe::new(&server);
    *probe.fail_put_containing.lock().expect("rule") = Some("冲突副本".into());
    let failed = sync_once(&b, &probe, "DEVB");
    assert!(
        !failed.errors.is_empty(),
        "the copy write must fail: {failed:?}"
    );

    let remote_has_a = remote_tree(&server).values().any(|v| *v == v_a);
    let local_has_a = vault_tree(&b).values().any(|v| *v == v_a);
    assert!(
        remote_has_a || local_has_a,
        "A's version was overwritten on the remote and is nowhere else"
    );

    // Once the server accepts writes again, the conflict resolves with both versions kept.
    let report = sync_clean(&b, &server, "DEVB");
    assert_eq!(report.conflicts.len(), 1, "{report:?}");
    assert!(
        vault_tree(&b).values().any(|v| *v == v_a),
        "A's version kept as the copy"
    );
    assert!(
        vault_tree(&b).values().any(|v| *v == v_b),
        "B's version kept"
    );
    sync_clean(&a, &server, "DEVA");
    assert_eq!(vault_tree(&a), vault_tree(&b));
    assert_eq!(remote_tree(&server), vault_tree(&b));
    let _ = fs::remove_dir_all(&base);
}

/// A plain upload run: no conflict may be reported (used where a second device did not change).
fn assert_no_conflict(report: &SyncReport) {
    assert!(
        report.conflicts.is_empty(),
        "unexpected conflict: {report:?}"
    );
}

/// A lost connection stops the run after three network failures in a row. Without this, every
/// remaining file would be tried, each with its own retries and connect timeouts.
#[test]
fn network_failure_stops_the_run_at_once() {
    let server = EtagStore::default();
    let base = std::env::temp_dir().join(format!("jiyue-e2e-{}", unique_id()));
    let a = base.join("A");
    fs::create_dir_all(&a).expect("A");
    for i in 0..50 {
        write(&a, &format!("n{i:02}.md"), format!("note {i}\n").as_bytes());
    }

    let probe = Probe::new(&server);
    probe.fail_from.store(10, Ordering::SeqCst);
    let report = sync_once(&a, &probe, "DEVA");
    assert!(
        report
            .fatal
            .as_deref()
            .is_some_and(|f| f.contains("network error")),
        "{report:?}"
    );
    // List (op 0), 9 good puts (ops 1..=9), then ops 10..=12 fail: the run stops after three.
    assert_eq!(report.errors.len(), 3, "{report:?}");
    assert_eq!(
        probe.ops.load(Ordering::SeqCst),
        13,
        "stopped right after the third failure"
    );
    assert!(report.uploaded.len() < 50);

    // The next run, with the server back, finishes the job without re-uploading.
    let rerun = sync_clean(&a, &server, "DEVA");
    assert_eq!(rerun.uploaded.len(), 50 - report.uploaded.len());
    assert_eq!(remote_tree(&server), vault_tree(&a));
    let _ = fs::remove_dir_all(&base);
}
