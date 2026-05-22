//! `wavelet clip ls / show / lineage` — clip-ref inspection (wb-n33n.6).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use wavelet::clipref::{walk_refs, ClipRef, WalkedRef};
use ulid::Ulid;

use super::super::ClipOp;

const SHORT_ULID_LEN: usize = 8;
const MIN_PREFIX_LEN: usize = 4;

/// Dispatch every `wavelet clip …` subcommand.
pub fn run(op: ClipOp) -> ExitCode {
    match op {
        ClipOp::Ls { workdir, kind, scene, tag, lineage } => {
            run_ls(workdir, kind, scene, tag, lineage)
        }
        ClipOp::Show { target, workdir } => run_show(target, workdir),
        ClipOp::Lineage { clip_id, workdir } => run_lineage(clip_id, workdir),
        ClipOp::Import { workdir, cache, dry_run } => {
            super::clip_import::run(workdir, cache, dry_run)
        }
    }
}

fn run_ls(
    workdir: Option<PathBuf>,
    kind: Option<String>,
    scene: Option<String>,
    tag: Option<String>,
    lineage: bool,
) -> ExitCode {
    let workdir = workdir.unwrap_or_else(|| PathBuf::from("."));
    let walked = match walk_refs(&workdir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("walk {}: {e}", workdir.display());
            return ExitCode::from(2);
        }
    };
    let filtered: Vec<_> = walked
        .into_iter()
        .filter(|w| matches_filters(&w.clip, kind.as_deref(), scene.as_deref(), tag.as_deref()))
        .collect();

    if lineage {
        print_lineage_forest(&filtered);
    } else {
        print_table(&filtered);
    }
    ExitCode::SUCCESS
}

fn run_show(target: String, workdir: Option<PathBuf>) -> ExitCode {
    let workdir = workdir.unwrap_or_else(|| PathBuf::from("."));
    let target_path = PathBuf::from(&target);
    let (path, clip) = if target_path.is_file() {
        match read_clip(&target_path) {
            Ok(c) => (target_path, c),
            Err(e) => {
                eprintln!("read {target}: {e}");
                return ExitCode::from(2);
            }
        }
    } else {
        match resolve_short(&target, &workdir) {
            Ok(w) => (w.path, w.clip),
            Err(msg) => {
                eprintln!("{msg}");
                return ExitCode::from(2);
            }
        }
    };
    println!("path: {}", path.display());
    println!("clip: {} ({})", clip.clip, short(&clip.clip));
    println!("kind: {}", clip.kind.as_kebab());
    println!("provider: {}", clip.provider);
    if let Some(m) = &clip.model {
        println!("model: {m}");
    }
    println!("asset: {}", clip.asset.display());
    println!("asset-hash: {}", clip.asset_hash);
    if let Some(s) = &clip.scene {
        println!("scene: {s}");
    }
    if let Some(p) = &clip.parent {
        println!("parent: {p} ({})", short(p));
    }
    if let Some(ek) = &clip.edit_kind {
        println!("edit-kind: {:?}", ek);
    }
    if let Some(ep) = &clip.edit_prompt {
        println!("edit-prompt: {ep}");
    }
    if let Some(c) = clip.cost_usd {
        println!("cost-usd: {c:.4}");
    }
    if !clip.tags.is_empty() {
        println!("tags: {}", clip.tags.join(", "));
    }
    println!("created-at: {}", clip.created_at);
    println!("prompt: {}", truncate(&clip.prompt, 200));
    ExitCode::SUCCESS
}

fn run_lineage(clip_id: String, workdir: Option<PathBuf>) -> ExitCode {
    let workdir = workdir.unwrap_or_else(|| PathBuf::from("."));
    let walked = match walk_refs(&workdir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("walk {}: {e}", workdir.display());
            return ExitCode::from(2);
        }
    };
    let target = match resolve_short_in(&clip_id, &walked) {
        Ok(t) => t,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };
    let by_id: BTreeMap<Ulid, &WalkedRef> =
        walked.iter().map(|w| (w.clip.clip, w)).collect();
    let mut by_parent: BTreeMap<Ulid, Vec<&WalkedRef>> = BTreeMap::new();
    for w in &walked {
        if let Some(parent) = w.clip.parent {
            by_parent.entry(parent).or_default().push(w);
        }
    }

    let root = climb_to_root(target, &by_id);
    println!("{}", format_node(&root.clip));
    print_descendants(&root.clip.clip, &by_parent, "");
    ExitCode::SUCCESS
}

fn matches_filters(
    clip: &ClipRef,
    kind: Option<&str>,
    scene: Option<&str>,
    tag: Option<&str>,
) -> bool {
    if let Some(k) = kind {
        if !clip.kind.as_kebab().eq_ignore_ascii_case(k) {
            return false;
        }
    }
    if let Some(s) = scene {
        match &clip.scene {
            Some(actual) => {
                if !actual.contains(s) {
                    return false;
                }
            }
            None => return false,
        }
    }
    if let Some(t) = tag {
        let needle = t.to_ascii_lowercase();
        let any = clip
            .tags
            .iter()
            .any(|x| x.to_ascii_lowercase().contains(&needle));
        if !any {
            return false;
        }
    }
    true
}

fn print_table(refs: &[WalkedRef]) {
    println!(
        "{:<8}  {:<16}  {:<6}  {:<60}  {:>8}  {:<8}  {}",
        "id", "kind", "scene", "prompt", "cost", "parent", "asset",
    );
    for w in refs {
        let c = &w.clip;
        println!(
            "{:<8}  {:<16}  {:<6}  {:<60}  {:>8}  {:<8}  {}",
            short(&c.clip),
            c.kind.as_kebab(),
            c.scene.as_deref().unwrap_or("-"),
            truncate(&c.prompt, 60),
            c.cost_usd
                .map(|v| format!("${v:.4}"))
                .unwrap_or_else(|| "-".into()),
            c.parent.map(|p| short(&p)).unwrap_or_else(|| "-".into()),
            asset_filename(&c.asset),
        );
    }
}

fn print_lineage_forest(refs: &[WalkedRef]) {
    let by_id: BTreeMap<Ulid, &WalkedRef> =
        refs.iter().map(|w| (w.clip.clip, w)).collect();
    let mut by_parent: BTreeMap<Ulid, Vec<&WalkedRef>> = BTreeMap::new();
    for w in refs {
        if let Some(parent) = w.clip.parent {
            by_parent.entry(parent).or_default().push(w);
        }
    }
    for w in refs {
        if w.clip.parent.is_none() || !by_id.contains_key(&w.clip.parent.unwrap()) {
            println!("{}", format_node(&w.clip));
            print_descendants(&w.clip.clip, &by_parent, "");
        }
    }
}

fn print_descendants(
    parent: &Ulid,
    by_parent: &BTreeMap<Ulid, Vec<&WalkedRef>>,
    prefix: &str,
) {
    let Some(children) = by_parent.get(parent) else { return };
    for (i, child) in children.iter().enumerate() {
        let last = i == children.len() - 1;
        let branch = if last { "└── " } else { "├── " };
        println!("{prefix}{branch}{}", format_node(&child.clip));
        let next_prefix = format!("{prefix}{}", if last { "    " } else { "│   " });
        print_descendants(&child.clip.clip, by_parent, &next_prefix);
    }
}

fn climb_to_root<'a>(
    start: &'a WalkedRef,
    by_id: &BTreeMap<Ulid, &'a WalkedRef>,
) -> &'a WalkedRef {
    let mut cur = start;
    while let Some(parent) = cur.clip.parent {
        match by_id.get(&parent) {
            Some(p) => cur = p,
            None => break,
        }
    }
    cur
}

fn format_node(clip: &ClipRef) -> String {
    let edit = clip
        .edit_kind
        .map(|e| format!(" [{e:?}]"))
        .unwrap_or_default();
    let cost = clip
        .cost_usd
        .map(|v| format!(" ${v:.4}"))
        .unwrap_or_default();
    format!(
        "{} {}{edit} — {}{cost}",
        short(&clip.clip),
        clip.kind.as_kebab(),
        truncate(&clip.prompt, 40),
    )
}

fn resolve_short(
    prefix: &str,
    workdir: &std::path::Path,
) -> Result<WalkedRef, String> {
    let walked = walk_refs(workdir).map_err(|e| format!("walk: {e}"))?;
    resolve_short_in(prefix, &walked).cloned()
}

fn resolve_short_in<'a>(
    prefix: &str,
    walked: &'a [WalkedRef],
) -> Result<&'a WalkedRef, String> {
    if prefix.len() < MIN_PREFIX_LEN {
        return Err(format!(
            "clip-id prefix too short ({} chars, need ≥{})",
            prefix.len(),
            MIN_PREFIX_LEN
        ));
    }
    let upper = prefix.to_ascii_uppercase();
    let matches: Vec<_> = walked
        .iter()
        .filter(|w| w.clip.clip.to_string().starts_with(&upper))
        .collect();
    match matches.len() {
        0 => Err(format!("no clip-ref matches prefix {prefix}")),
        1 => Ok(matches[0]),
        n => {
            let sample = matches
                .iter()
                .take(5)
                .map(|w| short(&w.clip.clip))
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "prefix {prefix} matches {n} clip-refs: {sample}{}",
                if n > 5 { ", …" } else { "" }
            ))
        }
    }
}

fn read_clip(path: &std::path::Path) -> Result<ClipRef, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let (clip, _body) = ClipRef::parse(&raw).map_err(|e| e.to_string())?;
    Ok(clip)
}

fn short(id: &Ulid) -> String {
    let s = id.to_string();
    s.chars().take(SHORT_ULID_LEN).collect()
}

fn truncate(s: &str, n: usize) -> String {
    let s = s.lines().next().unwrap_or(s);
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

fn asset_filename(p: &std::path::Path) -> String {
    p.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("-")
        .to_string()
}
