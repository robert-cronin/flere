//! Durable user workflow state, independent of whether a process is running.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Workflow {
    #[default]
    Todo,
    InProgress,
    NeedsMe,
    Waiting,
    Done,
}
impl Workflow {
    pub const ALL: [Self; 5] = [
        Self::NeedsMe,
        Self::InProgress,
        Self::Todo,
        Self::Waiting,
        Self::Done,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Todo => "Todo",
            Self::InProgress => "In progress",
            Self::NeedsMe => "Needs me",
            Self::Waiting => "Waiting",
            Self::Done => "Done",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in-progress",
            Self::NeedsMe => "needs-me",
            Self::Waiting => "waiting",
            Self::Done => "done",
        }
    }
    pub fn parse(s: &str) -> io::Result<Self> {
        Self::ALL
            .into_iter()
            .find(|v| v.key() == s)
            .ok_or_else(|| crate::wire::invalid("unknown workflow status"))
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CardMeta {
    pub status: Workflow,
    pub pinned: bool,
    // Historical wire/store field only; never grants privileges or forces a pin.
    #[serde(rename = "lead")]
    pub legacy_lead: bool,
    pub archived: bool,
    pub project: String,
    pub issue: String,
    pub pr: String,
    pub notes: String,
    pub branch: String,
    pub conversations: Vec<crate::native::Conversation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_conversation: Option<crate::native::Conversation>,
    pub operation: String,
    pub base_sha: String,
}
impl CardMeta {
    /// Borrow the complete legacy frontend projection without copying its notes
    /// or conversation history. Recency remains supervisor-owned saved state.
    pub(crate) fn snapshot_metadata(&self) -> impl Serialize + '_ {
        #[derive(Serialize)]
        struct View<'a> {
            status: &'a Workflow,
            pinned: &'a bool,
            #[serde(rename = "lead")]
            legacy_lead: &'a bool,
            archived: &'a bool,
            project: &'a str,
            issue: &'a str,
            pr: &'a str,
            notes: &'a str,
            branch: &'a str,
            conversations: &'a [crate::native::Conversation],
            operation: &'a str,
            base_sha: &'a str,
        }
        // Exhaustive destructuring makes a new metadata field require an
        // explicit wire-compatibility choice, instead of silently omitting it.
        let Self {
            status,
            pinned,
            legacy_lead,
            archived,
            project,
            issue,
            pr,
            notes,
            branch,
            conversations,
            last_conversation: _,
            operation,
            base_sha,
        } = self;
        View {
            status,
            pinned,
            legacy_lead,
            archived,
            project,
            issue,
            pr,
            notes,
            branch,
            conversations,
            operation,
            base_sha,
        }
    }
    pub fn recent_conversation(&self) -> Option<&crate::native::Conversation> {
        self.last_conversation
            .as_ref()
            .or(match self.conversations.as_slice() {
                [saved] => Some(saved),
                _ => None,
            })
    }
    pub fn validate(&self) -> io::Result<()> {
        for s in [&self.project, &self.issue, &self.pr, &self.branch] {
            if s.len() > 2048 || s.chars().any(char::is_control) {
                return Err(crate::wire::invalid(
                    "metadata contains controls or is too long",
                ));
            }
        }
        if self.conversations.iter().any(|c| {
            !crate::native::valid_uuid(&c.uuid)
                || !matches!(c.harness.as_str(), "codex" | "claude" | "copilot")
                || !c.cwd.is_absolute()
        }) {
            return Err(crate::wire::invalid("invalid saved native conversation"));
        }
        if self.last_conversation.as_ref().is_some_and(|c| {
            if c.uuid.is_empty() {
                !matches!(c.harness.as_str(), "codex" | "claude" | "copilot")
                    || !c.cwd.is_absolute()
            } else {
                !self.conversations.contains(c)
            }
        }) {
            return Err(crate::wire::invalid(
                "last native conversation is not recorded",
            ));
        }
        if self.notes.len() > 65536 {
            return Err(crate::wire::invalid("notes exceed 64 KiB"));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Inspector {
    #[default]
    Files,
    Git,
    Details,
}
impl Inspector {
    pub fn next(self) -> Self {
        match self {
            Self::Files => Self::Git,
            Self::Git => Self::Details,
            Self::Details => Self::Files,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Git => "Git",
            Self::Details => "Details",
        }
    }
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Grouping {
    #[default]
    Status,
    Project,
}
impl Grouping {
    pub fn label(self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Project => "Project",
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub left: usize,
    pub right: usize,
    pub inspector: Inspector,
    pub zoom: bool,
    pub compact_cards: bool,
    pub expanded_cards: Vec<u64>,
    pub reduced_motion: bool,
    pub screensaver_minutes: u16,
    pub screensaver_mascot: crate::pet::ScreensaverMascot,
    pub pet: bool,
    pub pet_kind: crate::pet::Kind,
    pub project: String,
    pub grouping: Grouping,
    pub folds: Vec<String>,
    pub history_epoch: String,
    pub history: Vec<(u64, u64, String)>,
    pub history_index: usize,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            left: 26,
            right: 40,
            inspector: Inspector::Files,
            zoom: false,
            compact_cards: false,
            expanded_cards: Vec::new(),
            reduced_motion: false,
            screensaver_minutes: 5,
            screensaver_mascot: crate::pet::ScreensaverMascot::default(),
            pet: false,
            pet_kind: crate::pet::Kind::default(),
            project: String::new(),
            grouping: Grouping::default(),
            folds: Vec::new(),
            history_epoch: String::new(),
            history: Vec::new(),
            history_index: 0,
        }
    }
}
pub fn load_preferences(state: &Path) -> Preferences {
    let mut prefs: Preferences = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(state.join("ui.json"))
        .ok()
        .and_then(|f| {
            if !f.metadata().ok()?.is_file() {
                return None;
            }
            let mut b = Vec::new();
            f.take(65537).read_to_end(&mut b).ok()?;
            if b.len() > 65536 {
                return None;
            }
            serde_json::from_slice(&b).ok()
        })
        .unwrap_or_default();
    prefs.expanded_cards.truncate(512);
    if !matches!(prefs.screensaver_minutes, 0 | 5 | 15) {
        prefs.screensaver_minutes = 5;
    }
    prefs
}
pub fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| crate::wire::invalid("missing parent"))?;
    let tmp = parent.join(format!(".{}.new", crate::os::nonce()?));
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(data)?;
        {
            let _timing = crate::diagnostics::measure("store-file-sync");
            f.sync_all()?;
        }
        fs::rename(&tmp, path)?;
        let _timing = crate::diagnostics::measure("store-directory-sync");
        File::open(parent)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
