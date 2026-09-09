//! File-backed record of a detached SCIP rebuild.
//!
//! `arbor scip --background` spawns a child process that outlives the invoking
//! shell, so the in-memory `TaskManager` in the MCP bridge cannot see it — the
//! two live in different processes. The handle therefore lives on disk at
//! `.arbor/scip-task.json`, which lets the CLI, the bridge, and an agent all
//! poll the same record.
//!
//! Writes go through a temp file and an atomic rename, matching how the graph
//! caches are persisted: a reader must never observe a half-written record.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where the record lives, relative to a project root.
pub fn task_path(project_root: &Path) -> PathBuf {
    project_root.join(".arbor").join("scip-task.json")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScipTaskStatus {
    Running,
    Completed,
    Failed,
}

impl ScipTaskStatus {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Running)
    }
}

impl std::fmt::Display for ScipTaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        };
        write!(f, "{}", s)
    }
}

/// One detached rebuild.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScipTask {
    pub id: String,
    pub status: ScipTaskStatus,

    /// `0`–`100`. Coarse on purpose: a Gradle build gives no usable progress
    /// signal, so reporting a fake percentage would be worse than three stages.
    pub progress: u8,

    pub message: String,

    /// PID of the detached worker, for diagnosing a run that never finishes.
    pub pid: u32,

    /// Full build log. Kept whole rather than summarised — a failed compile is
    /// the thing a user actually needs to read.
    pub log: String,

    pub started_at: u64,
    pub updated_at: u64,

    pub error: Option<String>,

    /// Populated on success, so a poller learns the outcome without having to
    /// re-query the graph.
    pub node_count: Option<usize>,
    pub edge_count: Option<usize>,
}

impl ScipTask {
    pub fn new(id: impl Into<String>, pid: u32, log: impl Into<String>) -> Self {
        let now = now_secs();
        Self {
            id: id.into(),
            status: ScipTaskStatus::Running,
            progress: 0,
            message: "Starting scip-java".to_string(),
            pid,
            log: log.into(),
            started_at: now,
            updated_at: now,
            error: None,
            node_count: None,
            edge_count: None,
        }
    }

    pub fn progress(mut self, progress: u8, message: impl Into<String>) -> Self {
        self.progress = progress.min(100);
        self.message = message.into();
        self.updated_at = now_secs();
        self
    }

    pub fn completed(mut self, node_count: usize, edge_count: usize) -> Self {
        self.status = ScipTaskStatus::Completed;
        self.progress = 100;
        self.message = format!("Ingested {} nodes, {} edges", node_count, edge_count);
        self.node_count = Some(node_count);
        self.edge_count = Some(edge_count);
        self.updated_at = now_secs();
        self
    }

    pub fn failed(mut self, error: impl Into<String>) -> Self {
        self.status = ScipTaskStatus::Failed;
        self.message = "Rebuild failed; the existing graph was left untouched".to_string();
        self.error = Some(error.into());
        self.updated_at = now_secs();
        self
    }

    /// Seconds since the record was last touched.
    pub fn age_secs(&self) -> u64 {
        now_secs().saturating_sub(self.updated_at)
    }

    /// Whether the worker process is still alive.
    ///
    /// A record left at `running` by a killed or interrupted worker would
    /// otherwise block every later rebuild. Asking the OS is the only correct
    /// answer — an age-based timeout is wrong in both directions: it locks the
    /// user out after a crash, and it lapses during a genuinely long build.
    ///
    /// Known limit: PIDs are recycled, so a long-dead worker whose PID has
    /// been reused reads as alive. The window is small and the failure mode is
    /// a spurious refusal, which `rm .arbor/scip-task.json` clears.
    pub fn worker_alive(&self) -> bool {
        if self.pid == 0 {
            return false;
        }

        #[cfg(unix)]
        {
            // Signal 0 performs error checking but sends nothing. Zero means
            // the process exists and we may signal it; EPERM means it exists
            // and we may not, which still counts as alive.
            let rc = unsafe { libc::kill(self.pid as libc::pid_t, 0) };
            if rc == 0 {
                return true;
            }
            std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
        }

        #[cfg(not(unix))]
        {
            // No cheap portable probe; assume alive and let the user clear the
            // record by hand rather than risk two concurrent Gradle builds.
            true
        }
    }

    /// Reads the record for a project, if one exists.
    ///
    /// A malformed file reads as absent rather than as an error: the record is
    /// diagnostic, and refusing to run because a status file is corrupt would
    /// be worse than ignoring it.
    pub fn load(project_root: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(task_path(project_root)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Writes the record atomically.
    pub fn save(&self, project_root: &Path) -> std::io::Result<()> {
        let final_path = task_path(project_root);
        if let Some(parent) = final_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let tmp = final_path.with_extension("json.tmp");
        let json =
            serde_json::to_string_pretty(self).map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &final_path)
    }

    /// JSON shaped like the MCP Tasks extension's `tasks/get` response, so a
    /// detached rebuild is pollable through the same call as an in-process task.
    pub fn to_task_response(&self) -> serde_json::Value {
        serde_json::json!({
            "taskId": self.id,
            "status": self.status.to_string(),
            "progress": self.progress,
            "message": self.message,
            "tool": "scip_rebuild",
            "error": self.error,
            "createdAt": self.started_at,
            "updatedAt": self.updated_at,
            "log": self.log,
            "nodeCount": self.node_count,
            "edgeCount": self.edge_count,
        })
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trips_through_disk() {
        let dir = TempDir::new().unwrap();
        let task = ScipTask::new("scip-1", 4242, "/tmp/x.log");
        task.save(dir.path()).unwrap();

        let loaded = ScipTask::load(dir.path()).expect("record must load");
        assert_eq!(loaded.id, "scip-1");
        assert_eq!(loaded.pid, 4242);
        assert_eq!(loaded.status, ScipTaskStatus::Running);
        assert!(!loaded.status.is_terminal());
    }

    #[test]
    fn absent_record_is_none_not_an_error() {
        let dir = TempDir::new().unwrap();
        assert!(ScipTask::load(dir.path()).is_none());
    }

    #[test]
    fn a_corrupt_record_reads_as_absent() {
        // Refusing to run because a diagnostic file is malformed would be
        // worse than ignoring it.
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".arbor")).unwrap();
        std::fs::write(task_path(dir.path()), "{not json").unwrap();
        assert!(ScipTask::load(dir.path()).is_none());
    }

    #[test]
    fn completion_records_the_outcome() {
        let task = ScipTask::new("scip-1", 1, "/tmp/x.log").completed(10596, 45543);
        assert_eq!(task.status, ScipTaskStatus::Completed);
        assert!(task.status.is_terminal());
        assert_eq!(task.progress, 100);
        assert_eq!(task.node_count, Some(10596));
        assert!(task.error.is_none());
    }

    #[test]
    fn failure_keeps_the_reason_and_says_the_graph_survived() {
        let task = ScipTask::new("scip-1", 1, "/tmp/x.log").failed("compile error");
        assert_eq!(task.status, ScipTaskStatus::Failed);
        assert_eq!(task.error.as_deref(), Some("compile error"));
        assert!(task.message.contains("left untouched"));
        assert!(task.node_count.is_none());
    }

    #[test]
    fn progress_is_clamped() {
        let task = ScipTask::new("scip-1", 1, "l").progress(250, "x");
        assert_eq!(task.progress, 100);
    }

    #[test]
    fn task_response_matches_the_mcp_shape() {
        let task = ScipTask::new("scip-7", 1, "/tmp/x.log").completed(1, 2);
        let v = task.to_task_response();
        assert_eq!(v["taskId"], "scip-7");
        assert_eq!(v["status"], "completed");
        assert_eq!(v["progress"], 100);
        assert_eq!(v["tool"], "scip_rebuild");
    }

    #[test]
    fn a_live_worker_reads_as_alive() {
        // This test's own process is unquestionably alive.
        let task = ScipTask::new("scip-1", std::process::id(), "l");
        assert!(task.worker_alive());
    }

    #[test]
    fn pid_zero_is_never_alive() {
        // A record written before the PID was known must not lock out rebuilds.
        let task = ScipTask::new("scip-1", 0, "l");
        assert!(!task.worker_alive());
    }

    #[cfg(unix)]
    #[test]
    fn a_reaped_worker_reads_as_dead() {
        // Spawn and reap a real process, then ask about its PID. An age-based
        // check would have reported this as still running.
        let child = std::process::Command::new("true")
            .spawn()
            .expect("spawn /usr/bin/true");
        let pid = child.id();
        let mut child = child;
        child.wait().expect("reap");

        let task = ScipTask::new("scip-1", pid, "l");
        assert!(
            !task.worker_alive(),
            "a reaped worker must not block later rebuilds"
        );
    }
}
