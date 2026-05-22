//! End-to-end smoke for the clip-ref pipeline (wb-n33n.10).
//!
//! Builds a tiny workdir entirely out of clip-refs, exercises the
//! producer-output → compose-pre-pass → inspection cycle, and asserts
//! lineage + idempotency. No real backends required; the test fabricates
//! manifests directly via `AssetCache::store_with_clip`.

use std::path::Path;

use wavelet::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use wavelet::backends::clipref_emit::ClipEmitContext;
use wavelet::clipref::{walk_refs, ClipKind, EditKind};
use wavelet::compose::{extract_audio_clip_cues, resolve_clip_refs};
use serde_json::json;

fn touch(p: &Path, body: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

fn mk_manifest(hash: &str, provider: &str, cluster: &str, ext: &str) -> Manifest {
    Manifest {
        version: 1,
        provider: provider.into(),
        cluster: cluster.into(),
        request_hash: hash.into(),
        request: json!({"prompt": "hand pouring water in a slow steady stream"}),
        response: json!({"job_id": "abc"}),
        cost_estimate_usd: 0.20,
        asset_path: Some(format!("{provider}/{hash}.{ext}")),
        created_at: utc_now_iso8601(),
    }
}

#[test]
fn full_pipeline_smoke_lineage_and_compose() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path().join("work");
    let cache_root = tmp.path().join("cache");
    std::fs::create_dir_all(&workdir).unwrap();

    let cache = AssetCache::new(&cache_root);

    // 1. emit a still
    touch(
        &cache_root.join("google/hashstill.png"),
        b"png-bytes",
    );
    let still_manifest =
        mk_manifest("hashstill", "google", "text_to_image", "png");
    let still_res = cache
        .store_with_clip(
            &still_manifest,
            ClipEmitContext {
                workdir: &workdir,
                kind: ClipKind::Still,
                prompt: "hand pouring water in a slow steady stream".into(),
                model: Some("nano-banana".into()),
                parent: None,
                edit_kind: None,
                edit_prompt: None,
                scene: Some("INT. KITCHEN - DAY".into()),
                tags: vec!["hero".into()],
            },
        )
        .unwrap();
    let still_raw = std::fs::read_to_string(&still_res.clipref_path).unwrap();
    let (still_clip, _) = wavelet::clipref::ClipRef::parse(&still_raw).unwrap();
    let still_id = still_clip.clip;

    // 2. emit a shot with the still as parent
    touch(
        &cache_root.join("google/hashshot.mp4"),
        b"mp4-bytes",
    );
    let shot_manifest =
        mk_manifest("hashshot", "google", "img2vid_gen", "mp4");
    let shot_res = cache
        .store_with_clip(
            &shot_manifest,
            ClipEmitContext {
                workdir: &workdir,
                kind: ClipKind::Shot,
                prompt: "subtle pour motion".into(),
                model: Some("veo-3.1-generate-preview".into()),
                parent: Some(still_id.to_string()),
                edit_kind: Some(EditKind::Regenerate),
                edit_prompt: None,
                scene: Some("INT. KITCHEN - DAY".into()),
                tags: vec!["hero".into()],
            },
        )
        .unwrap();
    let shot_raw = std::fs::read_to_string(&shot_res.clipref_path).unwrap();
    let (shot_clip, _) = wavelet::clipref::ClipRef::parse(&shot_raw).unwrap();

    // 3. lineage holds
    assert_eq!(shot_clip.parent, Some(still_id));
    assert_eq!(shot_clip.edit_kind, Some(EditKind::Regenerate));

    // 4. walk_refs sees both
    let walked = walk_refs(&workdir).unwrap();
    assert_eq!(walked.len(), 2, "still + shot");

    // 5. emit music
    touch(
        &cache_root.join("google/hashmusic.wav"),
        b"wav-bytes",
    );
    let music_manifest =
        mk_manifest("hashmusic", "google", "music_gen", "wav");
    cache
        .store_with_clip(
            &music_manifest,
            ClipEmitContext {
                workdir: &workdir,
                kind: ClipKind::Music,
                prompt: "calm ambient bed".into(),
                model: Some("lyria-3".into()),
                parent: None,
                edit_kind: None,
                edit_prompt: None,
                scene: None,
                tags: vec![],
            },
        )
        .unwrap();

    // 6. write a scene HTML that references the shot + the music via <wavelet-clip>
    let scene_dir = workdir.join("scenes");
    std::fs::create_dir_all(&scene_dir).unwrap();
    let shot_rel = pathdiff(&shot_res.clipref_path, &scene_dir);
    let music_rel = {
        let refs_dir = workdir.join("refs/music");
        let one = std::fs::read_dir(&refs_dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        pathdiff(&one, &scene_dir)
    };
    let scene_html = format!(
        r#"<html><body>
<wavelet-clip src="{shot}"></wavelet-clip>
<wavelet-clip src="{music}"></wavelet-clip>
<h1>title</h1>
</body></html>
"#,
        shot = shot_rel.display(),
        music = music_rel.display(),
    );
    let scene_path = scene_dir.join("hero.html");
    std::fs::write(&scene_path, &scene_html).unwrap();

    // 7. compose pre-pass substitutes the shot, drops the music
    let resolved = resolve_clip_refs(&scene_html, &scene_dir).unwrap();
    assert!(resolved.contains("<video"), "shot → <video>: {resolved}");
    assert!(!resolved.contains("<wavelet-clip"), "all consumed");
    assert!(!resolved.contains("<audio"), "music not inlined");

    // 8. extract_audio_clip_cues hoists the music cue
    let cues = extract_audio_clip_cues(&scene_html, &scene_dir, 30, 0, 90).unwrap();
    assert_eq!(cues.len(), 1, "music hoisted: {cues:?}");
    assert!(cues[0].asset_path.to_string_lossy().ends_with(".wav"));

    // 9. idempotent re-emission keeps the same paths
    let again = cache
        .store_with_clip(
            &shot_manifest,
            ClipEmitContext {
                workdir: &workdir,
                kind: ClipKind::Shot,
                prompt: "subtle pour motion".into(),
                model: Some("veo-3.1-generate-preview".into()),
                parent: Some(still_id.to_string()),
                edit_kind: Some(EditKind::Regenerate),
                edit_prompt: None,
                scene: Some("INT. KITCHEN - DAY".into()),
                tags: vec!["hero".into()],
            },
        )
        .unwrap();
    assert_eq!(again.clipref_path, shot_res.clipref_path);
}

fn pathdiff(target: &Path, base: &Path) -> std::path::PathBuf {
    use std::path::Component;
    let t = normalize(target);
    let b = normalize(base);
    let tc: Vec<_> = t.components().collect();
    let bc: Vec<_> = b.components().collect();
    let mut i = 0;
    while i < tc.len() && i < bc.len() && tc[i] == bc[i] {
        i += 1;
    }
    let mut out = std::path::PathBuf::new();
    for _ in i..bc.len() {
        out.push("..");
    }
    for c in &tc[i..] {
        if let Component::Normal(s) = c {
            out.push(s);
        }
    }
    out
}

fn normalize(p: &Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}
