//! The agent loop.
//!
//! `run_turn` is the heart of `wavelet agent`: it appends the user's
//! prompt to a session, calls Gemini with the tool declarations,
//! dispatches whichever functionCall parts come back, appends the
//! responses, and re-asks until the model returns plain text (or we
//! blow a budget / step cap).
//!
//! The orchestrator is synchronous on purpose — Gemini calls go
//! through the existing `ureq` client (see `edit/gemini.rs`), tool
//! dispatch is blocking subprocesses or local IO. The async surface
//! lives in `server.rs`, which calls `run_turn` from a `spawn_blocking`
//! task.

use serde_json::{json, Value};

use super::events::Event;
use super::plan::schema::Plan;
use super::session::{Session, ToolCallEntry};
use super::{AgentConfig, AgentError, AgentResult, PlanMode, ToolRegistry};
use crate::edit::gemini;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Hard sanity cap on loop iterations in `PlanMode::On`. We don't
/// consult `max_steps` in `On` (plan terminality + budget + wall-clock
/// drive termination instead) but a wildly broken model could still
/// burn money in a tight loop; this protects against that.
const MAX_RUNAWAY: u32 = 1000;

/// System-prompt addendum appended when `plan_mode != Off`.
const PLAN_PROMPT_ADDENDUM: &str = "
## Plan tools

This session uses a Plan. Read it with `plan.show`. Add tasks with `plan.add`.

**Cadence — non-negotiable.** After EACH tool call that completes the work \
for a task, IMMEDIATELY call `plan.complete` on that task in the same turn. \
Do not batch. Do not wait. Do not move on to the next task first.

Example: your task is 'Generate hero shot' and `wavelet.shot.txt2vid` just \
returned a path — your very next call is `plan.complete` on that task.

Validators run on `plan.complete`. If they fail, fix the work and call \
`plan.complete` again. Use `plan.fork` or `plan.reopen` to retry a different \
approach. When the plan is fully done, call `plan.done`.

Never end a turn with a Doing task left unmarked. The loop ends when the \
plan terminates or budget/time is exhausted.
";

/// Outcome of one orchestrator turn.
#[derive(Debug, Clone)]
pub enum TurnOutcome {
    /// Model returned a final text reply.
    Done,
    /// Loop ran out of steps without a final reply.
    StepsExhausted,
    /// Budget exhausted before the loop finished.
    BudgetExhausted,
}

/// Default system prompt — inlines the director crib so the agent
/// never has to hunt for an external SKILL.md from arbitrary cwds.
pub const DEFAULT_SYSTEM_PROMPT: &str = "\
You are `wavelet agent`, a Rust-native motion-graphics + video agent.
Your tools are the full `wavelet` CLI surface (rendering, shot edit,
txt2vid/img2vid, music gen, dialogue TTS, storyboard, continuity,
captions, query, c2pa) plus filesystem (`fs.*`) and web (`web.*`)
helpers. Always prefer one tool call over many small ones.

## Director crib

For a video / commercial / spot, the canonical pipeline is:
brief → shotlist → `txt2vid` (or `img2vid`) per shot → music gen → \
HTML compose → render.

For a multi-shot commercial: emit one `txt2vid` call per shot, save with \
descriptive names (e.g. `shots/shot-1.mp4`). Then author per-scene HTML \
files (one `.html` per shot, full HTML/CSS freedom for overlays — e.g. \
`scenes/01.html` containing `<video src=\"shots/shot-1.mp4\">` plus any \
creative markup). Finally write ONE top-level `commercial.html` \
manifest listing every scene + audio. Call `wavelet.render` with \
`comp: \"commercial.html\"` — it ingests HTML directly.

**Composition is HTML, never JSON.** Do not write comp.json — that \
path is deprecated. The `wavelet.render` tool reads the HTML manifest \
shape: `<section data-scene-href=\"scenes/01.html\" data-duration=\"5s\">` \
per scene, `<audio src=\"music/track.wav\" data-spans=\"all\">` per audio \
cue, with `<meta name=\"resolution\" content=\"1920x1080\">` / `fps` / \
`duration` in the `<head>`. The tool's description has a full example — \
read it before composing.

**Cost discipline.** Every paid backend call requires `max_cost` (USD); \
the default of $0 rejects. A typical Veo 6s shot costs ~$1.25 — budget \
accordingly. Pass `dry_run: true` first on any paid call to see the \
request spec without spending.

**Backend selection.** Omit `backend` to use the workdir \
`wavelet.config.toml` cascade default. Do not guess a backend name.

**Output paths.** Default to the current working directory unless the \
user gave one. Return paths verbatim in your final reply.

**Artifact placement.** Write artifacts (brief.md, script.fountain, \
shot-*.mp4, music.wav, etc.) to the WORKDIR root — never inside the \
`plan/` subdirectory. The `plan/` dir is exclusively for `.task.html` \
files; the plan tools manage it. Mixing artifacts in there will break \
later stages that look for them in the workdir.

**Brand grounding.** Before authoring the brief for a real-world brand, \
call `brand.brief` with the brand's domain. The response gives you \
their actual palette (use as CSS custom properties in scene HTML), real \
ad copy (mirror the tonal register), logos + product imagery. \
Generic-coffee-shop commercials are bad; brand-specific ones are good. \
Use `brand.fetch` if you only need identity (faster) and \
`brand.catalog` for product images. After `brand.brief`, use \
`brand.product` to pin down a specific SKU for hero shots, and \
`brand.ads` to study how the brand currently advertises (palette, \
copy voice, pacing).

Everything you need is in this prompt. Do not try to read external \
skill files.
";

/// Run a single agent turn end-to-end. Mutates `session` in place,
/// emits events via `emit`, and returns an `AgentResult` summarizing
/// the outcome.
pub fn run_turn(
    session: &mut Session,
    prompt: &str,
    tools: &ToolRegistry,
    config: &AgentConfig,
    emit: &dyn Fn(Event),
) -> Result<AgentResult, AgentError> {
    let started = Instant::now();
    let api_key = gemini::api_key_from_env().map_err(|_| AgentError::NoKey)?;

    initialize_plan(session, config)?;
    initialize_system_prompt(session, config);
    session.push_user(prompt);

    let declarations = tools.function_declarations();
    let mut note: Option<String> = None;
    let mut outcome = TurnOutcome::StepsExhausted;
    let mut final_text: Option<String> = None;
    let step_cap = match config.plan_mode {
        PlanMode::Off | PlanMode::Shadow => config.max_steps,
        PlanMode::On => MAX_RUNAWAY,
    };

    for step in 0..step_cap {
        if let Some((o, n)) = check_step_termination(session, config, started, &final_text) {
            outcome = o;
            note = n;
            break;
        }

        emit(Event::thinking("planning", step, session.cost_usd));

        let contents_json = Value::Array(session.contents.clone());
        let resp = gemini::generate_with_tools(
            &config.model,
            &contents_json,
            Some(&declarations),
            session.system_instruction.as_deref(),
            "high",
            &api_key,
        )
        .map_err(|e| AgentError::Gemini(e.to_string()))?;

        if let Some((pt, ot)) = resp.usage {
            session.record_gemini_cost(pt, ot, &config.model);
        }

        let calls: Vec<(String, Value, Option<String>)> = resp
            .function_calls()
            .into_iter()
            .map(|(n, a, sig)| (n.to_string(), a.clone(), sig.map(|s| s.to_string())))
            .collect();

        if calls.is_empty() {
            if let Some(o) = handle_text_response(session, emit, &resp, config, &mut final_text) {
                outcome = o;
                break;
            }
            continue;
        }

        session.push_assistant_function_calls(&calls);

        for (name, args, _sig) in &calls {
            dispatch_one_call(tools, session, emit, name, args, step);
        }
    }

    if matches!(outcome, TurnOutcome::StepsExhausted) && final_text.is_none() {
        note = Some(format!("step cap reached ({step_cap})"));
    }

    Ok(AgentResult {
        final_text,
        output_files: session.output_files.clone(),
        cost_usd: session.cost_usd,
        wall_ms: started.elapsed().as_millis(),
        session_id: session.id.clone(),
        note,
    })
}

fn initialize_plan(session: &mut Session, config: &AgentConfig) -> Result<(), AgentError> {
    if config.plan_mode == PlanMode::Off {
        return Ok(());
    }
    let plan_workdir = config.plan_workdir.clone().unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    });
    let mut outer = session.plan.lock().expect("plan cell poisoned");
    if outer.is_none() {
        let plan = Plan::load(&plan_workdir)
            .map_err(|e| AgentError::Gemini(format!("plan load: {e}")))?;
        *outer = Some(Arc::new(Mutex::new(plan)));
    }
    session.completion_signaled.store(false, Ordering::SeqCst);
    Ok(())
}

fn initialize_system_prompt(session: &mut Session, config: &AgentConfig) {
    if session.system_instruction.is_some() {
        return;
    }
    let mut sys = config
        .system_prompt
        .clone()
        .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string());
    if config.plan_mode != PlanMode::Off {
        sys.push_str(PLAN_PROMPT_ADDENDUM);
    }
    session.set_system(sys);
}

fn check_step_termination(
    session: &Session,
    config: &AgentConfig,
    started: Instant,
    final_text: &Option<String>,
) -> Option<(TurnOutcome, Option<String>)> {
    if session.cost_usd > config.max_cost_usd {
        let n = format!(
            "budget exhausted at ${:.4} (cap ${:.4})",
            session.cost_usd, config.max_cost_usd
        );
        return Some((TurnOutcome::BudgetExhausted, Some(n)));
    }
    if config.plan_mode != PlanMode::On {
        return None;
    }
    let elapsed_ms = started.elapsed().as_millis();
    let cap_ms = (config.max_wall_seconds as u128) * 1000;
    if elapsed_ms >= cap_ms {
        let n = format!("wall cap reached ({}s)", config.max_wall_seconds);
        return Some((TurnOutcome::BudgetExhausted, Some(n)));
    }
    if plan_is_terminal_no_failures(session) {
        return Some((TurnOutcome::Done, None));
    }
    if session.completion_signaled.load(Ordering::SeqCst) && final_text.is_some() {
        return Some((TurnOutcome::Done, None));
    }
    None
}

/// Handle a model response that returned text instead of function calls.
/// In Off/Shadow plan modes, text means we're done. In On mode, the model
/// can't bail mid-plan with a stray text reply — we only exit when plan is
/// terminal or completion was explicitly signaled.
fn handle_text_response(
    session: &mut Session,
    emit: &dyn Fn(Event),
    resp: &gemini::GeminiResponse,
    config: &AgentConfig,
    final_text: &mut Option<String>,
) -> Option<TurnOutcome> {
    let text = resp.text().unwrap_or_default();
    session.push_assistant_text(&text);
    emit(Event::final_text(text.clone(), session.cost_usd));
    *final_text = Some(text);
    match config.plan_mode {
        PlanMode::Off | PlanMode::Shadow => Some(TurnOutcome::Done),
        PlanMode::On => {
            if session.completion_signaled.load(Ordering::SeqCst)
                || plan_is_terminal_no_failures(session)
            {
                Some(TurnOutcome::Done)
            } else {
                None
            }
        }
    }
}

fn dispatch_one_call(
    tools: &ToolRegistry,
    session: &mut Session,
    emit: &dyn Fn(Event),
    name: &str,
    args: &Value,
    step: u32,
) {
    emit(Event::tool_call(
        name.to_string(),
        args.clone(),
        step,
        session.cost_usd,
    ));
    let Some(tool) = tools.get(name) else {
        let detail = format!("unknown tool `{name}`");
        emit(Event::tool_result(
            name.to_string(),
            false,
            detail.clone(),
            step,
            session.cost_usd,
        ));
        session.push_tool_response(name, &json!({ "error": detail.clone() }));
        session.record_tool_call(ToolCallEntry {
            name: name.to_string(),
            args: args.clone(),
            summary: detail,
            ok: false,
            cost_usd: 0.0,
        });
        return;
    };
    let result = tool.dispatch(args);
    emit(Event::tool_result(
        name.to_string(),
        result.ok,
        result.summary.clone(),
        step,
        session.cost_usd + result.cost_usd,
    ));
    for f in &result.output_files {
        if !session.output_files.contains(f) {
            session.output_files.push(f.clone());
        }
    }
    session.push_tool_response(name, &result.response);
    session.record_tool_call(ToolCallEntry {
        name: name.to_string(),
        args: args.clone(),
        summary: result.summary.clone(),
        ok: result.ok,
        cost_usd: result.cost_usd,
    });
}

/// True when the session's plan exists, every task is terminal, and no
/// task has a `validator_failure` marker on its body. We don't have an
/// in-memory "last validate pass" record, so we use the status alone —
/// `plan.complete` only flips to `Done` after validators pass, so a
/// fully-Done plan is the right signal here.
fn plan_is_terminal_no_failures(session: &Session) -> bool {
    let outer = match session.plan.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    let Some(inner) = outer.as_ref() else { return false };
    let inner_clone = inner.clone();
    drop(outer);
    let plan = match inner_clone.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    if plan.tasks.is_empty() {
        return false;
    }
    plan.is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::plan::schema::{Plan, Task, TaskId, TaskStatus};
    use crate::agent::plan::validator::ValidatorRegistry;
    use crate::agent::session::empty_completion_flag;
    use crate::agent::tools::{default_registry, plan_tools, ToolRegistry};
    use crate::pipelines::schema::StageSuccessCriterion;
    use chrono::Utc;
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::Mutex;

    fn no_emit(_: Event) {}

    // Build a Session + ToolRegistry pair whose plan/completion handles
    // are wired together, so `run_turn` can populate the cell and tools
    // (including `plan.done`) can mutate it. Avoids `AgentLoop` so we
    // don't pay for `default_registry()` in tests.
    fn wired(plan_workdir: &std::path::Path) -> (Session, ToolRegistry) {
        let plan = Plan::load(plan_workdir).unwrap();
        let inner = std::sync::Arc::new(Mutex::new(plan));
        let plan_cell: super::super::PlanCell =
            std::sync::Arc::new(Mutex::new(Some(inner)));
        let completion = empty_completion_flag();
        let mut reg = ToolRegistry::new();
        plan_tools::register_with_plan_and_completion(
            &mut reg,
            plan_cell.clone(),
            std::sync::Arc::new(ValidatorRegistry::with_builtins()),
            completion.clone(),
        );
        let session = Session::with_plan_handles(plan_cell, completion);
        (session, reg)
    }

    fn seed_done_task(workdir: &std::path::Path) {
        fs::write(workdir.join("brief.md"), b"hi\n").unwrap();
        let id = TaskId::new();
        let now = Utc::now();
        let task = Task {
            task: id,
            title: "draft".into(),
            status: TaskStatus::Done,
            description: None,
            deps: vec![],
            parent: None,
            budget_usd: None,
            budget_wall_s: None,
            validators: vec![StageSuccessCriterion {
                kind: "artifact_exists".into(),
                params: serde_yaml::from_str("{ path: brief.md }").unwrap(),
            }],
            created_at: now,
            updated_at: now,
            cost_usd: 0.0,
            attempts: 0,
            seed_from: None,
            extra: BTreeMap::new(),
        };
        task.write(&workdir.join(format!("{id}.task.html")), "\n").unwrap();
    }

    #[test]
    fn plan_mode_off_default_keeps_existing_session_shape() {
        // Regression: AgentConfig::default() yields Off, and a Session
        // built without wiring up a plan still parses cleanly.
        let cfg = AgentConfig::default();
        assert_eq!(cfg.plan_mode, PlanMode::Off);
        assert_eq!(cfg.max_wall_seconds, 1800);
        assert!(cfg.plan_workdir.is_none());

        let s = Session::new();
        let outer = s.plan.lock().unwrap();
        assert!(outer.is_none(), "default session should have plan cell off");
    }

    #[test]
    fn plan_mode_on_terminates_when_plan_already_terminal() {
        // Pre-seed a workdir whose plan is fully Done. `run_turn` should
        // detect terminality on the very first loop iteration before
        // calling Gemini. Budget cap is irrelevant — terminality wins.
        let dir = tempfile::tempdir().unwrap();
        seed_done_task(dir.path());
        let (mut session, reg) = wired(dir.path());
        // Reload after seeding so the plan cell sees the on-disk task.
        let fresh = Plan::load(dir.path()).unwrap();
        {
            let mut outer = session.plan.lock().unwrap();
            *outer = Some(std::sync::Arc::new(Mutex::new(fresh)));
        }
        // The tools were registered with the original (empty) cell; that's
        // fine for this test — we only need the orchestrator side.

        let cfg = AgentConfig {
            plan_mode: PlanMode::On,
            plan_workdir: Some(dir.path().to_path_buf()),
            max_steps: 100,
            max_cost_usd: 100.0,
            max_wall_seconds: 60,
            ..AgentConfig::default()
        };

        // Prime an API key so we don't bail on missing env; the test
        // should terminate before any network call.
        std::env::set_var("GOOGLE_API_KEY", "test-not-used");

        let result = run_turn(&mut session, "ignored", &reg, &cfg, &no_emit);
        assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
    }

    #[test]
    fn plan_mode_on_respects_max_cost_usd() {
        let dir = tempfile::tempdir().unwrap();
        let (mut session, reg) = wired(dir.path());
        session.cost_usd = 5.0; // over the cap below.

        let cfg = AgentConfig {
            plan_mode: PlanMode::On,
            plan_workdir: Some(dir.path().to_path_buf()),
            max_cost_usd: 0.001,
            max_wall_seconds: 60,
            ..AgentConfig::default()
        };
        std::env::set_var("GOOGLE_API_KEY", "test-not-used");

        let result = run_turn(&mut session, "ignored", &reg, &cfg, &no_emit).unwrap();
        let note = result.note.expect("expected a budget note");
        assert!(note.contains("budget"), "note was: {note}");
    }

    #[test]
    fn plan_mode_on_respects_max_wall_seconds() {
        let dir = tempfile::tempdir().unwrap();
        let (mut session, reg) = wired(dir.path());

        let cfg = AgentConfig {
            plan_mode: PlanMode::On,
            plan_workdir: Some(dir.path().to_path_buf()),
            max_cost_usd: 100.0,
            max_wall_seconds: 0, // first iteration's elapsed will exceed 0
            ..AgentConfig::default()
        };
        std::env::set_var("GOOGLE_API_KEY", "test-not-used");

        // Burn a tiny bit so elapsed > 0 by the time we hit the check.
        std::thread::sleep(std::time::Duration::from_millis(10));
        let result = run_turn(&mut session, "ignored", &reg, &cfg, &no_emit).unwrap();
        let note = result.note.expect("expected a wall-cap note");
        assert!(note.contains("wall"), "note was: {note}");
    }

    #[test]
    fn plan_mode_loads_plan_into_session_cell_on_init() {
        // Verifies the init hook: Shadow mode loads the plan into
        // `session.plan` and populates the inner Option. We use a
        // pre-emptive budget exhaustion to bail before any Gemini call.
        let dir = tempfile::tempdir().unwrap();
        seed_done_task(dir.path());
        let (mut session, reg) = wired(dir.path());
        // Reset the wired cell to None so we can prove `run_turn` loads.
        {
            let mut outer = session.plan.lock().unwrap();
            *outer = None;
        }
        session.cost_usd = 5.0;

        let cfg = AgentConfig {
            plan_mode: PlanMode::Shadow,
            plan_workdir: Some(dir.path().to_path_buf()),
            max_cost_usd: 0.001,
            max_steps: 24,
            ..AgentConfig::default()
        };
        std::env::set_var("GOOGLE_API_KEY", "test-not-used");

        let _ = run_turn(&mut session, "ignored", &reg, &cfg, &no_emit).unwrap();
        let outer = session.plan.lock().unwrap();
        assert!(outer.is_some(), "shadow mode should populate plan cell");
        let inner = outer.as_ref().unwrap().lock().unwrap();
        assert_eq!(inner.tasks.len(), 1, "plan should have the seeded task");
    }

    #[test]
    fn agent_loop_shares_plan_cell_with_session_and_tools() {
        // The PlanCell built by `AgentLoop::new` must be the same
        // identity-by-Arc as the one a session it spawns gets and the
        // one the `plan.*` tools hold. Flip-side check that the
        // completion flag is shared too.
        let agent = crate::agent::AgentLoop::new(AgentConfig::default());
        let session = agent.new_session();
        assert!(std::sync::Arc::ptr_eq(&agent.plan_cell, &session.plan));
        assert!(std::sync::Arc::ptr_eq(
            &agent.completion_signaled,
            &session.completion_signaled
        ));
    }

    #[test]
    fn session_records_user_and_tool_turns() {
        let mut s = Session::new();
        s.push_user("hello");
        s.push_assistant_function_calls(&[(
            "fs.list".to_string(),
            json!({"path": "."}),
            None,
        )]);
        s.push_tool_response("fs.list", &json!({"entries": []}));
        assert_eq!(s.contents.len(), 3);
        assert_eq!(s.contents[0]["role"], "user");
        assert_eq!(s.contents[1]["role"], "model");
        assert_eq!(s.contents[2]["role"], "user");
        assert_eq!(
            s.contents[1]["parts"][0]["functionCall"]["name"],
            "fs.list"
        );
    }

    #[test]
    fn registry_decl_includes_all_tools() {
        let r = default_registry();
        let decl = r.function_declarations();
        let fds = decl[0]["function_declarations"].as_array().unwrap();
        let names: Vec<&str> = fds
            .iter()
            .map(|fd| fd["name"].as_str().unwrap())
            .collect();
        for expected in [
            "wavelet.shot.edit",
            "wavelet.shot.txt2vid",
            "wavelet.shot.upscale",
            "wavelet.image.composite",
            "wavelet.image.isolate",
            "wavelet.music.gen",
            "wavelet.dialogue.tts",
            "wavelet.render",
            "verify",
            "workflow.run",
            "brief.check",
            "screenplay.parse",
            "velocity.propose",
            "velocity.validate",
            "storyboard.plan",
            "storyboard.verify",
            "continuity.check",
            "transitions.classify",
            "captions.align",
            "query.scene_graph",
            "query.pixels",
            "query.snapshot",
            "query.beat",
            "c2pa.sign",
            "c2pa.verify",
            "fs.read",
            "fs.write",
            "fs.list",
            "fs.exists",
            "fs.mkdir",
            "web.search",
            "web.fetch",
        ] {
            assert!(
                names.contains(&expected),
                "registry missing tool `{expected}`"
            );
        }
    }
}
