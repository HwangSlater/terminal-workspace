//! redb-backed storage implementations. See `docs/05-operations/storage.md`
//! for the schema and
//! `docs/06-development/decisions/0014-storage-engine-reconsideration.md`
//! for why the engine is `redb` (pure Rust, no C compiler required) rather
//! than SQLite.

use async_trait::async_trait;
use common::{Result, WorkspaceError};
use domain::{
    CacheRepository, FailedEventRecord, FailedEventRepository, MemberPresence, NotificationId,
    NotificationItem, NotificationRepository, PluginRepository, PresenceRepository,
    SettingsRepository, WorkspaceRepository,
};
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const NOTIFICATIONS: TableDefinition<&str, &[u8]> = TableDefinition::new("notifications");
const TEAM_PRESENCE: TableDefinition<&str, &[u8]> = TableDefinition::new("team_presence");
const KEY_VALUE_STORE: TableDefinition<&str, &[u8]> = TableDefinition::new("key_value_store");
const FAILED_EVENTS: TableDefinition<&str, &[u8]> = TableDefinition::new("failed_events");

/// redb-backed implementation of every domain repository contract.
pub struct RedbStorageBackend {
    db: Arc<Database>,
}

impl RedbStorageBackend {
    /// Open (creating if missing) the redb database at `path`.
    pub async fn open(path: &Path) -> Result<Self> {
        let path = path.to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| WorkspaceError::Storage(e.to_string()))?;
        }
        let db = run_blocking(move || Database::create(&path).storage_err()).await?;
        Ok(Self { db: Arc::new(db) })
    }
}

/// Resolve the OS-standard data directory path for `workspace.redb`. See
/// `docs/05-operations/storage.md` §2.
#[must_use]
pub fn standard_db_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());

    let mut path = PathBuf::from(home);
    if cfg!(windows) {
        path.push("AppData");
        path.push("Local");
    } else {
        path.push(".local");
        path.push("share");
    }
    path.push("terminal-workspace");
    path.push("workspace.redb");
    path
}

/// Maps any displayable error (redb's various error types, `serde_json`,
/// `tokio::task::JoinError`) into `WorkspaceError::Storage`.
trait StorageErr<T> {
    fn storage_err(self) -> Result<T>;
}

impl<T, E: std::fmt::Display> StorageErr<T> for std::result::Result<T, E> {
    fn storage_err(self) -> Result<T> {
        self.map_err(|e| WorkspaceError::Storage(e.to_string()))
    }
}

/// Run a synchronous redb closure on the blocking thread pool (redb's API
/// is synchronous; this keeps it off the async runtime's worker threads).
async fn run_blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f).await.storage_err()?
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn to_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).storage_err()
}

fn from_json<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).storage_err()
}

// -- Generic table operations, shared across all four tables. --------------

fn write_entry(
    db: &Database,
    table: TableDefinition<&str, &[u8]>,
    key: &str,
    bytes: &[u8],
) -> Result<()> {
    let write_txn = db.begin_write().storage_err()?;
    {
        let mut t = write_txn.open_table(table).storage_err()?;
        t.insert(key, bytes).storage_err()?;
    }
    write_txn.commit().storage_err()
}

fn read_entry(
    db: &Database,
    table: TableDefinition<&str, &[u8]>,
    key: &str,
) -> Result<Option<Vec<u8>>> {
    let read_txn = db.begin_read().storage_err()?;
    let t = read_txn.open_table(table).storage_err()?;
    Ok(t.get(key)
        .storage_err()?
        .map(|guard| guard.value().to_vec()))
}

fn scan_entries(
    db: &Database,
    table: TableDefinition<&str, &[u8]>,
) -> Result<Vec<(String, Vec<u8>)>> {
    let read_txn = db.begin_read().storage_err()?;
    let t = read_txn.open_table(table).storage_err()?;
    let mut out = Vec::new();
    for entry in t.iter().storage_err()? {
        let (k, v) = entry.storage_err()?;
        out.push((k.value().to_string(), v.value().to_vec()));
    }
    Ok(out)
}

fn remove_entry(db: &Database, table: TableDefinition<&str, &[u8]>, key: &str) -> Result<()> {
    let write_txn = db.begin_write().storage_err()?;
    {
        let mut t = write_txn.open_table(table).storage_err()?;
        t.remove(key).storage_err()?;
    }
    write_txn.commit().storage_err()
}

// -- NotificationRepository -------------------------------------------------

#[async_trait]
impl NotificationRepository for RedbStorageBackend {
    async fn save(&self, item: &NotificationItem) -> Result<()> {
        let db = Arc::clone(&self.db);
        let key = item.id.0.to_string();
        let bytes = to_json(item)?;
        run_blocking(move || write_entry(&db, NOTIFICATIONS, &key, &bytes)).await
    }

    async fn find_by_id(&self, id: &NotificationId) -> Result<Option<NotificationItem>> {
        let db = Arc::clone(&self.db);
        let key = id.0.to_string();
        run_blocking(move || match read_entry(&db, NOTIFICATIONS, &key)? {
            Some(bytes) => Ok(Some(from_json(&bytes)?)),
            None => Ok(None),
        })
        .await
    }

    async fn fetch_unread(&self) -> Result<Vec<NotificationItem>> {
        let db = Arc::clone(&self.db);
        run_blocking(move || {
            let mut items: Vec<NotificationItem> = scan_entries(&db, NOTIFICATIONS)?
                .into_iter()
                .map(|(_, bytes)| from_json(&bytes))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .filter(|item: &NotificationItem| !item.is_read)
                .collect();
            items.sort_by_key(|item| std::cmp::Reverse(item.timestamp_ms));
            Ok(items)
        })
        .await
    }

    async fn mark_read(&self, id: &NotificationId) -> Result<()> {
        let db = Arc::clone(&self.db);
        let key = id.0.to_string();
        run_blocking(move || {
            if let Some(bytes) = read_entry(&db, NOTIFICATIONS, &key)? {
                let mut item: NotificationItem = from_json(&bytes)?;
                item.is_read = true;
                write_entry(&db, NOTIFICATIONS, &key, &to_json(&item)?)?;
            }
            Ok(())
        })
        .await
    }
}

// -- PresenceRepository -------------------------------------------------

#[async_trait]
impl PresenceRepository for RedbStorageBackend {
    async fn save_presence(&self, presence: &MemberPresence) -> Result<()> {
        let db = Arc::clone(&self.db);
        let key = presence.user_id.0.clone();
        let bytes = to_json(presence)?;
        run_blocking(move || write_entry(&db, TEAM_PRESENCE, &key, &bytes)).await
    }

    async fn fetch_all(&self) -> Result<Vec<MemberPresence>> {
        let db = Arc::clone(&self.db);
        run_blocking(move || {
            scan_entries(&db, TEAM_PRESENCE)?
                .into_iter()
                .map(|(_, bytes)| from_json(&bytes))
                .collect()
        })
        .await
    }
}

// -- key_value_store-backed repositories: Settings / Cache / Workspace / Plugin --

/// Value shape stored in `key_value_store` (see `docs/05-operations/storage.md`).
#[derive(Serialize, Deserialize)]
struct KvEntry {
    value: String,
    expires_at: Option<u64>,
}

fn set_kv(db: &Database, key: &str, value: &str, expires_at: Option<u64>) -> Result<()> {
    let entry = KvEntry {
        value: value.to_string(),
        expires_at,
    };
    write_entry(db, KEY_VALUE_STORE, key, &to_json(&entry)?)
}

fn get_kv(db: &Database, key: &str) -> Result<Option<String>> {
    match read_entry(db, KEY_VALUE_STORE, key)? {
        Some(bytes) => Ok(Some(from_json::<KvEntry>(&bytes)?.value)),
        None => Ok(None),
    }
}

/// Like [`get_kv`], but treats an entry whose `expires_at` has passed as
/// absent, lazily deleting it.
fn get_kv_with_expiry(db: &Database, key: &str) -> Result<Option<String>> {
    match read_entry(db, KEY_VALUE_STORE, key)? {
        Some(bytes) => {
            let entry: KvEntry = from_json(&bytes)?;
            match entry.expires_at {
                Some(expires_at) if expires_at <= now_ms() / 1000 => {
                    remove_entry(db, KEY_VALUE_STORE, key)?;
                    Ok(None)
                }
                _ => Ok(Some(entry.value)),
            }
        }
        None => Ok(None),
    }
}

#[async_trait]
impl SettingsRepository for RedbStorageBackend {
    async fn get_value(&self, key: &str) -> Result<Option<String>> {
        let db = Arc::clone(&self.db);
        let key = format!("setting:{key}");
        run_blocking(move || get_kv(&db, &key)).await
    }

    async fn set_value(&self, key: &str, value: &str) -> Result<()> {
        let db = Arc::clone(&self.db);
        let key = format!("setting:{key}");
        let value = value.to_string();
        run_blocking(move || set_kv(&db, &key, &value, None)).await
    }
}

#[async_trait]
impl CacheRepository for RedbStorageBackend {
    async fn set_cache(&self, key: &str, value: &str, ttl_secs: u64) -> Result<()> {
        let db = Arc::clone(&self.db);
        let key = format!("cache:{key}");
        let value = value.to_string();
        let expires_at = now_ms() / 1000 + ttl_secs;
        run_blocking(move || set_kv(&db, &key, &value, Some(expires_at))).await
    }

    async fn get_cache(&self, key: &str) -> Result<Option<String>> {
        let db = Arc::clone(&self.db);
        let key = format!("cache:{key}");
        run_blocking(move || get_kv_with_expiry(&db, &key)).await
    }
}

#[async_trait]
impl WorkspaceRepository for RedbStorageBackend {
    async fn save_layout(&self, layout_json: &str) -> Result<()> {
        let db = Arc::clone(&self.db);
        let layout_json = layout_json.to_string();
        run_blocking(move || set_kv(&db, "workspace:layout", &layout_json, None)).await
    }

    async fn load_layout(&self) -> Result<Option<String>> {
        let db = Arc::clone(&self.db);
        run_blocking(move || get_kv(&db, "workspace:layout")).await
    }
}

#[async_trait]
impl PluginRepository for RedbStorageBackend {
    async fn save_plugin_manifest(&self, plugin_id: &str, manifest_json: &str) -> Result<()> {
        let db = Arc::clone(&self.db);
        let key = format!("plugin:{plugin_id}");
        let manifest_json = manifest_json.to_string();
        run_blocking(move || set_kv(&db, &key, &manifest_json, None)).await
    }

    async fn get_active_plugins(&self) -> Result<Vec<(String, String)>> {
        let db = Arc::clone(&self.db);
        run_blocking(move || {
            scan_entries(&db, KEY_VALUE_STORE)?
                .into_iter()
                .filter_map(|(key, bytes)| {
                    key.strip_prefix("plugin:").map(|plugin_id| {
                        from_json::<KvEntry>(&bytes)
                            .map(|entry| (plugin_id.to_string(), entry.value))
                    })
                })
                .collect()
        })
        .await
    }
}

// -- FailedEventRepository (Dead Letter Queue) -------------------------------

#[async_trait]
impl FailedEventRepository for RedbStorageBackend {
    async fn save_failed(&self, record: &FailedEventRecord) -> Result<()> {
        let db = Arc::clone(&self.db);
        let key = record.id.to_string();
        let bytes = to_json(record)?;
        run_blocking(move || write_entry(&db, FAILED_EVENTS, &key, &bytes)).await
    }

    async fn list_failed(&self) -> Result<Vec<FailedEventRecord>> {
        let db = Arc::clone(&self.db);
        run_blocking(move || {
            let mut records: Vec<FailedEventRecord> = scan_entries(&db, FAILED_EVENTS)?
                .into_iter()
                .map(|(_, bytes)| from_json(&bytes))
                .collect::<Result<Vec<_>>>()?;
            records.sort_by_key(|record| std::cmp::Reverse(record.failed_at_ms));
            Ok(records)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{IntegrationSource, PresenceStatus, PriorityLevel, UserId};
    use uuid::Uuid;

    async fn open_temp_backend() -> RedbStorageBackend {
        let path = std::env::temp_dir().join(format!("tw_test_{}.redb", Uuid::new_v4()));
        RedbStorageBackend::open(&path)
            .await
            .expect("temp db should open")
    }

    fn sample_notification() -> NotificationItem {
        NotificationItem {
            id: NotificationId(Uuid::new_v4()),
            source: IntegrationSource::Slack,
            title: "Build Succeeded".into(),
            body: "All checks passed.".into(),
            timestamp_ms: 1_716_373_200_000,
            priority: PriorityLevel::High,
            is_read: false,
            action_link: Some("https://example.com".into()),
        }
    }

    #[tokio::test]
    async fn notification_round_trip() {
        let backend = open_temp_backend().await;
        let item = sample_notification();

        backend.save(&item).await.unwrap();
        let fetched = backend.find_by_id(&item.id).await.unwrap().unwrap();
        assert_eq!(fetched.title, item.title);
        assert_eq!(fetched.priority, PriorityLevel::High);
        assert!(!fetched.is_read);

        let unread = backend.fetch_unread().await.unwrap();
        assert_eq!(unread.len(), 1);

        backend.mark_read(&item.id).await.unwrap();
        let unread_after = backend.fetch_unread().await.unwrap();
        assert!(unread_after.is_empty());
    }

    #[tokio::test]
    async fn presence_round_trip() {
        let backend = open_temp_backend().await;
        let presence = MemberPresence {
            user_id: UserId("u1".into()),
            display_name: "Alice".into(),
            status: PresenceStatus::Meeting,
            custom_status_text: Some("In standup".into()),
            last_updated_ms: 123,
        };

        backend.save_presence(&presence).await.unwrap();
        let all = backend.fetch_all().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].status, PresenceStatus::Meeting);

        // Upsert on the same user_id must replace, not duplicate.
        backend
            .save_presence(&MemberPresence {
                status: PresenceStatus::Offline,
                ..presence
            })
            .await
            .unwrap();
        let all_after = backend.fetch_all().await.unwrap();
        assert_eq!(all_after.len(), 1);
        assert_eq!(all_after[0].status, PresenceStatus::Offline);
    }

    #[tokio::test]
    async fn settings_round_trip() {
        let backend = open_temp_backend().await;
        assert_eq!(backend.get_value("theme").await.unwrap(), None);
        backend.set_value("theme", "nord").await.unwrap();
        assert_eq!(
            backend.get_value("theme").await.unwrap(),
            Some("nord".to_string())
        );
    }

    #[tokio::test]
    async fn workspace_layout_round_trip() {
        let backend = open_temp_backend().await;
        assert_eq!(backend.load_layout().await.unwrap(), None);
        backend.save_layout("{\"panes\":[]}").await.unwrap();
        assert_eq!(
            backend.load_layout().await.unwrap(),
            Some("{\"panes\":[]}".to_string())
        );
    }

    #[tokio::test]
    async fn plugin_manifest_round_trip() {
        let backend = open_temp_backend().await;
        backend
            .save_plugin_manifest("pomodoro-timer", "{\"version\":1}")
            .await
            .unwrap();
        let active = backend.get_active_plugins().await.unwrap();
        assert_eq!(
            active,
            vec![("pomodoro-timer".to_string(), "{\"version\":1}".to_string())]
        );
    }

    #[tokio::test]
    async fn cache_expires_after_ttl() {
        let backend = open_temp_backend().await;
        backend.set_cache("k", "v", 0).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        assert_eq!(backend.get_cache("k").await.unwrap(), None);
    }

    #[tokio::test]
    async fn cache_available_within_ttl() {
        let backend = open_temp_backend().await;
        backend.set_cache("k", "v", 3600).await.unwrap();
        assert_eq!(backend.get_cache("k").await.unwrap(), Some("v".to_string()));
    }

    #[tokio::test]
    async fn failed_event_round_trip() {
        let backend = open_temp_backend().await;
        let record = FailedEventRecord {
            id: Uuid::new_v4(),
            event_type: "SlackMessageReceived".into(),
            producer: "notification-handler".into(),
            payload_json: "{}".into(),
            error_message: "handler panicked".into(),
            retry_count: 5,
            failed_at_ms: now_ms(),
        };

        backend.save_failed(&record).await.unwrap();
        let all = backend.list_failed().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].retry_count, 5);
    }
}
