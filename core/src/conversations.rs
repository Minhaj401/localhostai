//! Coordinator-owned conversations. Revisions prevent lost updates; a per-turn
//! file lease prevents two processes from generating into the same conversation.
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MESSAGES: usize = 1000;
const MAX_CONVERSATIONS: usize = 500;
const MAX_STORE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: u64,
    pub model: Option<String>,
    #[serde(default)]
    pub interrupted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub revision: u64,
    pub model: Option<String>,
    pub ctx: u32,
    pub system_prompt: String,
    pub active_request: Option<String>,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub id: String,
    pub title: String,
    pub updated_at: u64,
    pub revision: u64,
    pub model: Option<String>,
    pub message_count: usize,
    pub generating: bool,
    pub preview: String,
}

impl Conversation {
    pub fn summary(&self) -> Summary {
        Summary {
            id: self.id.clone(),
            title: self.title.clone(),
            updated_at: self.updated_at,
            revision: self.revision,
            model: self.model.clone(),
            message_count: self.messages.len(),
            generating: self.active_request.is_some(),
            preview: self
                .messages
                .last()
                .map(|m| m.content.chars().take(160).collect())
                .unwrap_or_default(),
        }
    }

    pub fn prompt(&self) -> Vec<serde_json::Value> {
        let budget = (self.ctx as usize * 3 * 3 / 4).clamp(1024, 3 * 1024 * 1024);
        let mut used = self.system_prompt.len() + 32;
        let mut messages = Vec::new();
        // The last assistant placeholder is not part of the next prompt.
        for message in self.messages.iter().rev().filter(|m| !m.content.is_empty()) {
            if used + message.content.len() + 32 > budget {
                break;
            }
            used += message.content.len() + 32;
            messages.push(serde_json::json!({"role":message.role,"content":message.content}));
        }
        if !self.system_prompt.is_empty() {
            messages.push(serde_json::json!({"role":"system","content":self.system_prompt}));
        }
        messages.reverse();
        messages
    }

    pub fn markdown(&self) -> String {
        let mut out = format!("# {}\n\n", self.title.replace(['\n', '\r'], " "));
        if let Some(model) = &self.model {
            out.push_str(&format!("Model: {model}\n\n"));
        }
        for message in &self.messages {
            out.push_str(&format!(
                "## {}{}\n\n{}\n\n",
                if message.role == "user" {
                    "You"
                } else if message.role == "system" {
                    "System"
                } else {
                    "Assistant"
                },
                if message.interrupted {
                    " (interrupted)"
                } else {
                    ""
                },
                message.content
            ));
        }
        out
    }
}

#[derive(Debug)]
pub struct StoreError(pub u16, pub String);
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.1.fmt(f)
    }
}
impl std::error::Error for StoreError {}
fn failure(code: u16, message: &str) -> anyhow::Error {
    StoreError(code, message.into()).into()
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn new_id() -> Result<String> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

type SummaryCache = std::collections::HashMap<String, (std::time::SystemTime, u64, Summary)>;

#[derive(Clone)]
pub struct ConversationStore {
    root: PathBuf,
    pub id: String,
    summaries: std::sync::Arc<std::sync::Mutex<SummaryCache>>,
}

impl ConversationStore {
    pub fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        }
        let lock = private_file(&root.join("store.lock"))?;
        lock.lock_exclusive()?;
        let identity = root.join("identity");
        let id = if identity.exists() {
            fs::read_to_string(identity)?
        } else {
            let id = new_id()?;
            let mut file = private_file(&identity)?;
            file.write_all(id.as_bytes())?;
            file.sync_all()?;
            id
        };
        if !valid_id(&id) {
            bail!("invalid conversation store identity");
        }
        Ok(Self {
            root,
            id,
            summaries: Default::default(),
        })
    }

    fn lock(&self) -> Result<File> {
        let file = private_file(&self.root.join("store.lock"))?;
        file.lock_exclusive()?;
        Ok(file)
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            return Err(failure(400, "invalid conversation id"));
        }
        Ok(self.root.join(format!("{id}.json")))
    }
    fn lease(&self, id: &str) -> Result<File> {
        self.path(id)?;
        let file = private_file(&self.root.join(format!("{id}.turn")))?;
        file.try_lock_exclusive().map_err(|_| {
            failure(
                409,
                "A reply is already running in this conversation. Refresh or stop it first.",
            )
        })?;
        Ok(file)
    }
    fn read(&self, id: &str) -> Result<Conversation> {
        let path = self.path(id)?;
        let file = File::open(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                failure(404, "conversation not found")
            } else {
                e.into()
            }
        })?;
        if file.metadata()?.len() > MAX_BYTES {
            bail!("conversation exceeds storage limit");
        }
        let mut conversation: Conversation = serde_json::from_reader(file)?;
        // A dropped/crashed process releases its OS lease. Never leave an old
        // partial reply labelled as running after restart.
        if conversation.active_request.is_some() {
            if let Ok(_lease) = self.lease(id) {
                conversation.active_request = None;
                self.write(&mut conversation)?;
            }
        }
        Ok(conversation)
    }
    fn write(&self, conversation: &mut Conversation) -> Result<()> {
        conversation.revision += 1;
        conversation.updated_at = now_ms().max(conversation.updated_at + 1);
        let bytes = serde_json::to_vec(conversation)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(failure(
                413,
                "Conversation is full. Export it and start a new chat.",
            ));
        }
        let destination = self.path(&conversation.id)?;
        let total: u64 = self
            .entries()?
            .iter()
            .filter(|p| **p != destination)
            .filter_map(|p| p.metadata().ok())
            .map(|m| m.len())
            .sum();
        if total + bytes.len() as u64 > MAX_STORE_BYTES {
            return Err(failure(
                507,
                "Conversation library storage is full. Export and delete chats to make room.",
            ));
        }
        let temporary = self.root.join(format!("{}.pending", conversation.id));
        let mut file = private_file(&temporary)?;
        file.set_len(0)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(temporary, destination)?;
        #[cfg(unix)]
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }
    fn entries(&self) -> Result<Vec<PathBuf>> {
        Ok(fs::read_dir(&self.root)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect())
    }
    pub fn list(&self, query: &str) -> Result<Vec<Summary>> {
        if query.len() > 512 {
            return Err(failure(400, "search is too long"));
        }
        let _lock = self.lock()?;
        let query = query.to_lowercase();
        let mut results = Vec::new();
        let mut summaries = self.summaries.lock().unwrap_or_else(|e| e.into_inner());
        let mut seen = std::collections::HashSet::new();
        for path in self.entries()? {
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            seen.insert(id.to_string());
            let metadata = path.metadata()?;
            let modified = metadata.modified()?;
            if query.is_empty() {
                if let Some((at, size, summary)) = summaries.get(id) {
                    if *at == modified && *size == metadata.len() && !summary.generating {
                        results.push(summary.clone());
                        continue;
                    }
                }
            }
            let conversation = self
                .read(id)
                .with_context(|| format!("read conversation {id}"))?;
            let summary = conversation.summary();
            summaries.insert(id.to_string(), (modified, metadata.len(), summary.clone()));
            if query.is_empty()
                || conversation.title.to_lowercase().contains(&query)
                || conversation
                    .messages
                    .iter()
                    .any(|m| m.content.to_lowercase().contains(&query))
            {
                results.push(summary);
            }
        }
        summaries.retain(|id, _| seen.contains(id));
        results.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
        Ok(results)
    }
    pub fn get(&self, id: &str) -> Result<Conversation> {
        let _lock = self.lock()?;
        self.read(id)
    }
    pub fn create(&self, title: &str) -> Result<Conversation> {
        self.import(title, Vec::new())
    }

    /// Explicit phone-history import; never automatically sends cloud chats to a laptop.
    pub fn import(&self, title: &str, messages: Vec<Message>) -> Result<Conversation> {
        if messages.len() > MAX_MESSAGES
            || messages.iter().any(|m| {
                !matches!(m.role.as_str(), "user" | "assistant" | "system")
                    || m.content.len() > 512 * 1024
            })
        {
            return Err(failure(400, "Invalid imported conversation messages"));
        }
        let title = checked_title(title)?;
        let _lock = self.lock()?;
        let entries = self.entries()?;
        let total: u64 = entries
            .iter()
            .filter_map(|p| p.metadata().ok())
            .map(|m| m.len())
            .sum();
        if entries.len() >= MAX_CONVERSATIONS || total >= MAX_STORE_BYTES {
            return Err(failure(
                507,
                "Conversation library is full. Export and delete chats to make room.",
            ));
        }
        let now = now_ms();
        let mut conversation = Conversation {
            id: new_id()?,
            title,
            created_at: now,
            updated_at: now,
            revision: 0,
            model: None,
            ctx: 4096,
            system_prompt: String::new(),
            active_request: None,
            messages,
        };
        self.write(&mut conversation)?;
        Ok(conversation)
    }
    pub fn rename(&self, id: &str, revision: u64, title: &str) -> Result<Conversation> {
        let title = checked_title(title)?;
        let _lock = self.lock()?;
        let mut conversation = self.read(id)?;
        check_revision(&conversation, revision)?;
        conversation.title = title;
        self.write(&mut conversation)?;
        Ok(conversation)
    }
    pub fn delete(&self, id: &str, revision: u64) -> Result<()> {
        let _lock = self.lock()?;
        let conversation = self.read(id)?;
        check_revision(&conversation, revision)?;
        let _lease = self.lease(id)?;
        fs::remove_file(self.path(id)?)?;
        Ok(())
    }
    pub fn cancel(&self, id: &str, request_id: &str) -> Result<Conversation> {
        let _lock = self.lock()?;
        let mut conversation = self.read(id)?;
        if conversation.active_request.as_deref() == Some(request_id) {
            conversation.active_request = None;
            self.write(&mut conversation)?;
        }
        Ok(conversation)
    }
    pub fn begin(&self, id: &str, turn: TurnRequest) -> Result<(TurnLease, Conversation)> {
        if !valid_id(&turn.request_id) {
            return Err(failure(400, "invalid request id"));
        }
        if !(512..=32768).contains(&turn.ctx)
            || turn.system_prompt.len() > turn.ctx as usize / 2
            || turn.content.trim().is_empty()
            || turn.content.len() > turn.ctx as usize * 3 / 2
        {
            return Err(failure(
                400,
                "Message or system prompt exceeds the selected context (512–32768 tokens).",
            ));
        }
        let _lock = self.lock()?;
        let mut conversation = self.read(id)?;
        check_revision(&conversation, turn.revision)?;
        let lease = self.lease(id)?;
        if conversation.messages.len() + 2 > MAX_MESSAGES
            || serde_json::to_vec(&conversation)?.len() > MAX_BYTES as usize / 2
        {
            return Err(failure(
                413,
                "Conversation is full. Export it and start a new chat.",
            ));
        }
        if conversation
            .messages
            .iter()
            .any(|m| m.id == format!("{}-user", turn.request_id))
        {
            return Err(failure(
                409,
                "This request was already saved. Refresh the conversation before retrying.",
            ));
        }
        if conversation.messages.is_empty() && conversation.title == "New chat" {
            conversation.title = turn
                .content
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(80)
                .collect();
        }
        conversation.model = turn.model.clone();
        conversation.ctx = turn.ctx;
        conversation.system_prompt = turn.system_prompt;
        conversation.active_request = Some(turn.request_id.clone());
        for (role, content) in [("user", turn.content), ("assistant", String::new())] {
            conversation.messages.push(Message {
                id: format!("{}-{role}", turn.request_id),
                role: role.into(),
                content,
                created_at: now_ms(),
                model: turn.model.clone(),
                interrupted: role == "assistant",
            });
        }
        self.write(&mut conversation)?;
        Ok((
            TurnLease {
                store: self.clone(),
                id: id.into(),
                request_id: turn.request_id,
                _lease: lease,
                finished: false,
            },
            conversation,
        ))
    }
}

pub struct TurnRequest {
    pub revision: u64,
    pub request_id: String,
    pub content: String,
    pub model: Option<String>,
    pub ctx: u32,
    pub system_prompt: String,
}
pub struct TurnLease {
    store: ConversationStore,
    id: String,
    request_id: String,
    _lease: File,
    finished: bool,
}
impl TurnLease {
    pub fn checkpoint(&mut self, text: &str, complete: bool) -> Result<()> {
        let _lock = self.store.lock()?;
        let mut conversation = self.store.read(&self.id)?;
        if conversation.active_request.as_deref() != Some(&self.request_id) {
            return Err(failure(409, "Generation was stopped from another device."));
        }
        if text.len() > 512 * 1024 {
            return Err(failure(
                413,
                "Reply exceeds the conversation storage limit.",
            ));
        }
        if let Some(message) = conversation.messages.last_mut() {
            if message.content == text && !complete {
                return Ok(());
            }
            message.content = text.into();
            message.interrupted = !complete;
        }
        if complete {
            conversation.active_request = None;
        }
        self.store.write(&mut conversation)?;
        self.finished = complete;
        Ok(())
    }
}
impl Drop for TurnLease {
    fn drop(&mut self) {
        if !self.finished {
            if let Err(error) = self.store.cancel(&self.id, &self.request_id) {
                tracing::warn!(%error, "could not finish interrupted conversation");
            }
        }
    }
}
fn check_revision(conversation: &Conversation, revision: u64) -> Result<()> {
    if conversation.revision != revision {
        return Err(failure(409, "This conversation changed on another device. Refresh it before retrying; your draft is retained."));
    }
    Ok(())
}
fn checked_title(title: &str) -> Result<String> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 120 || title.chars().any(char::is_control) {
        return Err(failure(
            400,
            "Use a conversation name between 1 and 120 characters.",
        ));
    }
    Ok(title.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn turn(revision: u64) -> TurnRequest {
        TurnRequest {
            revision,
            request_id: new_id().unwrap(),
            content: "Explain Unicode 日本語".into(),
            model: Some("model".into()),
            ctx: 2048,
            system_prompt: String::new(),
        }
    }
    #[test]
    fn shared_store_prevents_lost_updates_and_recovers_interrupted_turns() {
        let dir = tempfile::tempdir().unwrap();
        let a = ConversationStore::open(dir.path().into()).unwrap();
        let b = ConversationStore::open(dir.path().into()).unwrap();
        let chat = a.create("New chat").unwrap();
        let (mut lease, active) = a.begin(&chat.id, turn(chat.revision)).unwrap();
        assert!(b.begin(&chat.id, turn(active.revision)).is_err());
        lease.checkpoint("Partial 🙂", false).unwrap();
        let copy = b.get(&chat.id).unwrap();
        assert!(copy.active_request.is_some());
        assert!(b.rename(&chat.id, chat.revision, "Old edit").is_err());
        drop(lease);
        let reopened = ConversationStore::open(dir.path().into())
            .unwrap()
            .get(&chat.id)
            .unwrap();
        assert!(reopened.active_request.is_none());
        assert!(reopened.messages[1].interrupted);
        assert_eq!(reopened.messages[1].content, "Partial 🙂");
        assert_eq!(b.list("日本語").unwrap().len(), 1);
        assert!(reopened.markdown().contains("Partial 🙂"));
        assert!(b.get("../config").is_err());
        b.delete(&chat.id, reopened.revision).unwrap();
        assert!(a.list("").unwrap().is_empty());
    }
}
