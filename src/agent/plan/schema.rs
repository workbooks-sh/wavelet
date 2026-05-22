//! Plan schema: on-disk shape (`<workdir>/*.task.html`) + in-memory aggregator.
//!
//! The validator type is reused from `crate::pipelines::schema` —
//! grading a task's outcome is the same problem as grading a pipeline
//! stage. Forward-compat unknown front-matter keys round-trip through
//! `Task::extra`, mirroring `ClipRef::extra`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::pipelines::schema::StageSuccessCriterion;

/// ULID-backed identifier for a `Task`. Newtype so we can vary the
/// representation later without touching call sites.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TaskId(pub Ulid);

impl TaskId {
    /// Mint a fresh ULID. Lexicographic order = creation order.
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Lifecycle state of a `Task`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    /// Not started.
    Todo,
    /// Actively being worked.
    Doing,
    /// Validators passed.
    Done,
    /// Cannot proceed (waiting on external input or an unresolved dep).
    Blocked,
    /// Will not be completed. Terminal but unsuccessful.
    Abandoned,
}

impl TaskStatus {
    /// `Done` or `Abandoned` — nothing further will happen here.
    pub fn is_terminal(&self) -> bool {
        matches!(self, TaskStatus::Done | TaskStatus::Abandoned)
    }
}

/// Errors produced by Plan/Task parse / write / load.
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    /// Input did not start with a `---` YAML front-matter delimiter.
    #[error("front matter missing or malformed")]
    FrontMatterMissing,
    /// Closing `---` delimiter not found after the opening one.
    #[error("body missing front-matter delimiter")]
    BodyMissingDelimiter,
    /// YAML decode failed.
    #[error("yaml decode: {0}")]
    YamlDecode(#[from] serde_yaml::Error),
    /// A `parent` pointer references a task that does not exist.
    #[error("task {child} has unknown parent {parent}")]
    UnknownParent {
        /// Child task ID.
        child: TaskId,
        /// Missing parent.
        parent: TaskId,
    },
    /// A `deps` edge forms a cycle.
    #[error("cycle in task deps involving {0}")]
    DepCycle(TaskId),
    /// Numeric invariant violated.
    #[error("invalid task {task}: {reason}")]
    InvalidTask {
        /// Offending task.
        task: TaskId,
        /// Human-readable reason.
        reason: String,
    },
    /// Filesystem error on read/write/load.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// One task in the plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct Task {
    /// Primary identifier. ULIDs sort lexicographically by creation time.
    pub task: TaskId,
    /// One-line human-readable title.
    pub title: String,
    /// Lifecycle state.
    pub status: TaskStatus,

    /// Longer human-readable description.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,

    /// Tasks that must reach a terminal good state before this one runs.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub deps: Vec<TaskId>,

    /// Parent task — set when this task forks from another.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parent: Option<TaskId>,

    /// USD budget for this task.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub budget_usd: Option<f32>,

    /// Wall-time budget in seconds.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub budget_wall_s: Option<u32>,

    /// Validators (reused from pipeline stage success criteria).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub validators: Vec<StageSuccessCriterion>,

    /// ISO-8601 UTC creation time.
    pub created_at: DateTime<Utc>,
    /// ISO-8601 UTC last-update time.
    pub updated_at: DateTime<Utc>,

    /// USD spent so far on this task.
    #[serde(default)]
    pub cost_usd: f32,
    /// Number of attempts made on this task.
    #[serde(default)]
    pub attempts: u32,

    /// Optional seed pointer (e.g. `commercial.yaml/publish`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub seed_from: Option<String>,

    /// Forward-compat catch-all. Unknown YAML keys land here and survive
    /// round-trip. Kebab-cased keys are preserved as-is.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

impl Task {
    /// Parse a `.task.html` file's contents. Returns the `Task` and the
    /// HTML body (everything after the closing `---`, including the
    /// leading newline).
    ///
    /// TODO(wb-mqsb): consolidate this front-matter splitter with the
    /// one in `crate::clipref` — they're identical aside from the
    /// concrete type. Trivially shareable once a second consumer lands.
    pub fn parse(html: &str) -> Result<(Task, String), PlanError> {
        let rest = html
            .strip_prefix("---\n")
            .or_else(|| html.strip_prefix("---\r\n"))
            .ok_or(PlanError::FrontMatterMissing)?;

        let (yaml, body) = split_on_closing_delimiter(rest)
            .ok_or(PlanError::BodyMissingDelimiter)?;

        let task: Task = serde_yaml::from_str(yaml)?;
        task.validate_numeric()?;
        Ok((task, body.to_string()))
    }

    /// Write a `.task.html` file. Front matter is serialized YAML; body
    /// is appended verbatim. Round-trips `extra` through `#[serde(flatten)]`.
    pub fn write(&self, path: &Path, body: &str) -> Result<(), PlanError> {
        self.validate_numeric()?;
        let yaml = serde_yaml::to_string(self)?;
        let mut out = String::with_capacity(yaml.len() + body.len() + 16);
        out.push_str("---\n");
        out.push_str(&yaml);
        if !yaml.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("---");
        out.push_str(body);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, out)?;
        Ok(())
    }

    fn validate_numeric(&self) -> Result<(), PlanError> {
        if self.cost_usd < 0.0 {
            return Err(PlanError::InvalidTask {
                task: self.task,
                reason: format!("cost_usd must be >= 0, got {}", self.cost_usd),
            });
        }
        if let Some(b) = self.budget_usd {
            if b < 0.0 {
                return Err(PlanError::InvalidTask {
                    task: self.task,
                    reason: format!("budget_usd must be >= 0, got {b}"),
                });
            }
        }
        Ok(())
    }
}

/// In-memory plan aggregator. Maps `TaskId` → `Task`, plus the workdir
/// the plan was loaded from (used for write-through on `insert` /
/// `update` / `remove`).
#[derive(Debug, Clone)]
pub struct Plan {
    /// All tasks in the plan, keyed by ID.
    pub tasks: HashMap<TaskId, Task>,
    /// Workdir — directory holding the `*.task.html` files.
    pub workdir: PathBuf,
}

impl Plan {
    /// Load every `*.task.html` directly under `workdir`. Missing
    /// workdir → empty plan. Validates parent pointers and rejects dep
    /// cycles after the full load.
    pub fn load(workdir: &Path) -> Result<Self, PlanError> {
        let mut tasks: HashMap<TaskId, Task> = HashMap::new();

        if workdir.exists() {
            for entry in fs::read_dir(workdir)? {
                let entry = entry?;
                let path = entry.path();
                if !path.to_string_lossy().ends_with(".task.html") {
                    continue;
                }
                let raw = fs::read_to_string(&path)?;
                let (task, _body) = Task::parse(&raw)?;
                tasks.insert(task.task, task);
            }
        }

        validate_parents(&tasks)?;
        validate_dep_cycles(&tasks)?;

        Ok(Self {
            tasks,
            workdir: workdir.to_path_buf(),
        })
    }

    /// Insert a task and write it through to disk. Replaces any
    /// existing task with the same ID.
    pub fn insert(&mut self, task: Task, body: &str) -> Result<(), PlanError> {
        let path = self.path_for(task.task);
        task.write(&path, body)?;
        self.tasks.insert(task.task, task);
        Ok(())
    }

    /// Replace an existing task's front matter (keeping the body that
    /// the caller supplies). Equivalent to `insert` — preserved as a
    /// distinct verb so call sites read clearly.
    pub fn update(&mut self, task: Task, body: &str) -> Result<(), PlanError> {
        self.insert(task, body)
    }

    /// Remove a task from disk and the in-memory map.
    pub fn remove(&mut self, id: TaskId) -> Result<Option<Task>, PlanError> {
        let path = self.path_for(id);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        Ok(self.tasks.remove(&id))
    }

    /// Topological sort of tasks by `deps`. Display-only — the agent is
    /// not bound to execute in this order. Ties broken by ULID order
    /// for stable output. Returns tasks even if cycles exist (cycles
    /// are rejected at `load`, so this path is unreachable for
    /// well-formed plans; if a runtime hand-build introduces one, the
    /// cycle nodes get appended at the end in ULID order).
    pub fn topo_sort(&self) -> Vec<TaskId> {
        let mut indegree: HashMap<TaskId, usize> = HashMap::new();
        let mut children: HashMap<TaskId, Vec<TaskId>> = HashMap::new();

        for (id, task) in &self.tasks {
            indegree.entry(*id).or_insert(0);
            for dep in &task.deps {
                if self.tasks.contains_key(dep) {
                    *indegree.entry(*id).or_insert(0) += 1;
                    children.entry(*dep).or_default().push(*id);
                }
            }
        }

        let mut ready: Vec<TaskId> = indegree
            .iter()
            .filter_map(|(id, n)| if *n == 0 { Some(*id) } else { None })
            .collect();
        ready.sort();

        let mut out = Vec::with_capacity(self.tasks.len());
        while let Some(id) = ready.pop() {
            out.push(id);
            if let Some(kids) = children.get(&id) {
                let mut newly_ready = Vec::new();
                for child in kids {
                    if let Some(n) = indegree.get_mut(child) {
                        *n -= 1;
                        if *n == 0 {
                            newly_ready.push(*child);
                        }
                    }
                }
                newly_ready.sort();
                for c in newly_ready {
                    ready.insert(0, c);
                }
            }
        }

        if out.len() < self.tasks.len() {
            let mut rest: Vec<TaskId> = self
                .tasks
                .keys()
                .copied()
                .filter(|id| !out.contains(id))
                .collect();
            rest.sort();
            out.extend(rest);
        }
        out
    }

    /// True iff every task is `Done` or `Abandoned`. Empty plan = true.
    pub fn is_terminal(&self) -> bool {
        self.tasks.values().all(|t| t.status.is_terminal())
    }

    fn path_for(&self, id: TaskId) -> PathBuf {
        self.workdir.join(format!("{id}.task.html"))
    }
}

fn validate_parents(tasks: &HashMap<TaskId, Task>) -> Result<(), PlanError> {
    for task in tasks.values() {
        if let Some(parent) = task.parent {
            if !tasks.contains_key(&parent) {
                return Err(PlanError::UnknownParent {
                    child: task.task,
                    parent,
                });
            }
        }
    }
    Ok(())
}

fn validate_dep_cycles(tasks: &HashMap<TaskId, Task>) -> Result<(), PlanError> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Visiting,
        Done,
    }

    let mut marks: HashMap<TaskId, Mark> = HashMap::new();

    fn dfs(
        node: TaskId,
        tasks: &HashMap<TaskId, Task>,
        marks: &mut HashMap<TaskId, Mark>,
    ) -> Result<(), PlanError> {
        match marks.get(&node) {
            Some(Mark::Done) => return Ok(()),
            Some(Mark::Visiting) => return Err(PlanError::DepCycle(node)),
            None => {}
        }
        marks.insert(node, Mark::Visiting);
        if let Some(task) = tasks.get(&node) {
            for dep in &task.deps {
                if tasks.contains_key(dep) {
                    dfs(*dep, tasks, marks)?;
                }
            }
        }
        marks.insert(node, Mark::Done);
        Ok(())
    }

    let mut keys: Vec<TaskId> = tasks.keys().copied().collect();
    keys.sort();
    for id in keys {
        if !matches!(marks.get(&id), Some(Mark::Done)) {
            dfs(id, tasks, &mut marks)?;
        }
    }
    Ok(())
}

/// Locate the closing `---` delimiter on its own line. Returns
/// `(yaml_before, body_after_including_leading_newline)`.
fn split_on_closing_delimiter(input: &str) -> Option<(&str, &str)> {
    let mut search_from = 0usize;
    while let Some(idx) = input[search_from..].find("---") {
        let abs = search_from + idx;
        let starts_line = abs == 0 || input.as_bytes()[abs - 1] == b'\n';
        let after = abs + 3;
        let ends_line = after == input.len()
            || matches!(input.as_bytes()[after], b'\n' | b'\r');
        if starts_line && ends_line {
            return Some((&input[..abs], &input[after..]));
        }
        search_from = abs + 3;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn ulid(s: &str) -> TaskId {
        TaskId(Ulid::from_str(s).unwrap())
    }

    fn sample_task(id: TaskId) -> Task {
        Task {
            task: id,
            title: "draft brief".into(),
            status: TaskStatus::Todo,
            description: Some("write a 200-word brief".into()),
            deps: vec![],
            parent: None,
            budget_usd: Some(0.50),
            budget_wall_s: Some(120),
            validators: vec![StageSuccessCriterion {
                kind: "artifact_exists".into(),
                params: serde_yaml::from_str("{ path: brief.md }").unwrap(),
            }],
            created_at: DateTime::parse_from_rfc3339("2026-05-20T14:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339("2026-05-20T14:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
            cost_usd: 0.0,
            attempts: 0,
            seed_from: Some("commercial.yaml/brief".into()),
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn round_trip_single_task() {
        let task = sample_task(ulid("01JQX9NXFVR2D5JBQGFCWQHZNX"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("01JQX9NXFVR2D5JBQGFCWQHZNX.task.html");
        let body = "\n\n<p>brief notes</p>\n";
        task.write(&path, body).unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        let (parsed, parsed_body) = Task::parse(&raw).unwrap();
        assert_eq!(parsed, task);
        assert_eq!(parsed_body, body);
    }

    #[test]
    fn unknown_front_matter_keys_survive() {
        let raw = "---\n\
            task: 01JQX9NXFVR2D5JBQGFCWQHZNX\n\
            title: experimental\n\
            status: todo\n\
            created-at: \"2026-05-20T14:30:00Z\"\n\
            updated-at: \"2026-05-20T14:30:00Z\"\n\
            future-knob: 42\n\
            another-future: hello\n\
            ---\n\
            <p>body</p>\n";

        let (task, _body) = Task::parse(raw).unwrap();
        assert_eq!(
            task.extra.get("future-knob"),
            Some(&serde_yaml::Value::Number(42.into()))
        );
        assert_eq!(
            task.extra.get("another-future"),
            Some(&serde_yaml::Value::String("hello".into()))
        );

        let yaml = serde_yaml::to_string(&task).unwrap();
        assert!(yaml.contains("future-knob: 42"));
        assert!(yaml.contains("another-future: hello"));
    }

    #[test]
    fn plan_load_with_parent_pointer_succeeds() {
        let root = ulid("01JQX0000000000000000000AA");
        let mid = ulid("01JQX0000000000000000000BB");
        let leaf = ulid("01JQX0000000000000000000CC");

        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();

        let mut t_root = sample_task(root);
        t_root.title = "root".into();

        let mut t_mid = sample_task(mid);
        t_mid.title = "mid".into();
        t_mid.parent = Some(root);
        t_mid.deps = vec![root];

        let mut t_leaf = sample_task(leaf);
        t_leaf.title = "leaf".into();
        t_leaf.deps = vec![mid];

        t_root.write(&workdir.join(format!("{root}.task.html")), "\n").unwrap();
        t_mid.write(&workdir.join(format!("{mid}.task.html")), "\n").unwrap();
        t_leaf.write(&workdir.join(format!("{leaf}.task.html")), "\n").unwrap();

        let plan = Plan::load(workdir).unwrap();
        assert_eq!(plan.tasks.len(), 3);
        assert_eq!(plan.tasks.get(&mid).unwrap().parent, Some(root));

        let order = plan.topo_sort();
        let pos = |id: TaskId| order.iter().position(|x| *x == id).unwrap();
        assert!(pos(root) < pos(mid));
        assert!(pos(mid) < pos(leaf));

        assert!(!plan.is_terminal());
    }

    #[test]
    fn unknown_parent_rejected() {
        let child = ulid("01JQX0000000000000000000XX");
        let missing = ulid("01JQX0000000000000000000YY");

        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();

        let mut t = sample_task(child);
        t.parent = Some(missing);
        t.write(&workdir.join(format!("{child}.task.html")), "\n").unwrap();

        match Plan::load(workdir) {
            Err(PlanError::UnknownParent { .. }) => {}
            other => panic!("expected UnknownParent, got {other:?}"),
        }
    }

    #[test]
    fn dep_cycle_rejected() {
        let a = ulid("01JQX0000000000000000000A1");
        let b = ulid("01JQX0000000000000000000B1");

        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();

        let mut ta = sample_task(a);
        ta.deps = vec![b];
        let mut tb = sample_task(b);
        tb.deps = vec![a];

        ta.write(&workdir.join(format!("{a}.task.html")), "\n").unwrap();
        tb.write(&workdir.join(format!("{b}.task.html")), "\n").unwrap();

        match Plan::load(workdir) {
            Err(PlanError::DepCycle(_)) => {}
            other => panic!("expected DepCycle, got {other:?}"),
        }
    }

    #[test]
    fn validator_matches_commercial_yaml_shape() {
        let yaml = "kind: cost_below_usd\nparams:\n  max: 5.0\n";
        let crit: StageSuccessCriterion = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(crit.kind, "cost_below_usd");

        let inline = "kind: artifact_exists\nparams: { path: brief.md }\n";
        let crit2: StageSuccessCriterion = serde_yaml::from_str(inline).unwrap();
        assert_eq!(crit2.kind, "artifact_exists");

        let id = ulid("01JQX9NXFVR2D5JBQGFCWQHZNX");
        let mut t = sample_task(id);
        t.validators = vec![crit, crit2];

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.task.html");
        t.write(&path, "\n").unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        let (parsed, _) = Task::parse(&raw).unwrap();
        assert_eq!(parsed.validators.len(), 2);
        assert_eq!(parsed.validators[0].kind, "cost_below_usd");
    }

    #[test]
    fn negative_cost_rejected_on_parse() {
        let raw = "---\n\
            task: 01JQX9NXFVR2D5JBQGFCWQHZNX\n\
            title: bad\n\
            status: todo\n\
            cost-usd: -1.0\n\
            created-at: \"2026-05-20T14:30:00Z\"\n\
            updated-at: \"2026-05-20T14:30:00Z\"\n\
            ---\n";
        match Task::parse(raw) {
            Err(PlanError::InvalidTask { .. }) => {}
            other => panic!("expected InvalidTask, got {other:?}"),
        }
    }

    #[test]
    fn empty_workdir_loads_empty_plan() {
        let dir = tempfile::tempdir().unwrap();
        let plan = Plan::load(dir.path()).unwrap();
        assert!(plan.tasks.is_empty());
        assert!(plan.is_terminal());
    }

    #[test]
    fn insert_and_remove_round_trips_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut plan = Plan::load(dir.path()).unwrap();
        let id = ulid("01JQX0000000000000000000Z1");
        let t = sample_task(id);
        plan.insert(t, "\nbody\n").unwrap();

        let reloaded = Plan::load(dir.path()).unwrap();
        assert!(reloaded.tasks.contains_key(&id));

        plan.remove(id).unwrap();
        let reloaded2 = Plan::load(dir.path()).unwrap();
        assert!(!reloaded2.tasks.contains_key(&id));
    }
}
