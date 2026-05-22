use std::process::ExitCode;
use crate::handlers::util::{INSERT_INTO_SCENE_INSTRUCTION, INSERT_INTO_SCENE_MAX_RETRIES};
use crate::handlers::util::InsertIntoSceneArgs;
use crate::handlers::util::image_arg_to_url;
use crate::handlers::util::resolve_local_path;
use crate::handlers::util::short_digest;

/// (auto-generated placeholder)
pub fn run_shot_insert_into_scene(a: InsertIntoSceneArgs) -> ExitCode {
    use crate::backends::fal::{FalClient, FalClipSimilarityAdapter, FalKontextMaxAdapter};
    use crate::backends::image::{
        IdentityCheckRequest, IdentitySimilarityBackend, InstructionEditBackend,
        InstructionEditRequest,
    };
    use crate::backends::roboflow::RoboflowClipAdapter;
    use crate::backends::{fal::RoboflowClient, BackendError, RunMode};
    use crate::image_analysis::concat::concat_horizontal;

    if a.backend != "fal-flux-kontext-max" {
        eprintln!(
            "unknown --backend '{}', want fal-flux-kontext-max",
            a.backend
        );
        return ExitCode::from(3);
    }
    if a.identity_backend != "roboflow-clip" && a.identity_backend != "fal-clip-similarity" {
        eprintln!(
            "unknown --identity-backend '{}', want roboflow-clip|fal-clip-similarity",
            a.identity_backend
        );
        return ExitCode::from(3);
    }

    let product_local = match resolve_local_path(&a.product) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("shot insert-into-scene --product: {e}");
            return ExitCode::from(2);
        }
    };
    let scene_local = match resolve_local_path(&a.scene) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("shot insert-into-scene --scene: {e}");
            return ExitCode::from(2);
        }
    };

    let concat_dir = a.cache.join("insert-into-scene");
    if let Err(e) = std::fs::create_dir_all(&concat_dir) {
        eprintln!("shot insert-into-scene: prepare cache dir: {e}");
        return ExitCode::from(2);
    }
    let concat_name = format!(
        "concat-{}.png",
        short_digest(&format!("{}|{}", product_local.display(), scene_local.display()))
    );
    let concat_path = concat_dir.join(concat_name);
    let concat_out = match concat_horizontal(&product_local, &scene_local, &concat_path) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("shot insert-into-scene: concat: {e}");
            return ExitCode::from(2);
        }
    };

    let concat_url = match image_arg_to_url(concat_path.to_str().unwrap_or("")) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("shot insert-into-scene: encode concat: {e}");
            return ExitCode::from(2);
        }
    };
    let product_url = match image_arg_to_url(&a.product) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("shot insert-into-scene --product: {e}");
            return ExitCode::from(2);
        }
    };

    let mode = if a.dry_run {
        RunMode::DryRun
    } else {
        RunMode::Live {
            max_cost_usd: a.max_cost,
        }
    };

    let client = if a.dry_run {
        FalClient::with_key("dry-run", &a.cache)
    } else {
        match FalClient::from_env(&a.cache) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("backend fal-flux-kontext-max: {e}");
                if let BackendError::MissingCredential(name) = &e {
                    eprintln!("set {name} or pass --dry-run to preview.");
                }
                return ExitCode::from(2);
            }
        }
    };
    let kontext = FalKontextMaxAdapter::new(client);

    let identity_adapter: Option<Box<dyn IdentitySimilarityBackend>> = if a.dry_run {
        None
    } else {
        match a.identity_backend.as_str() {
            "roboflow-clip" => {
                let rf_client = match RoboflowClient::from_env(&a.cache) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("backend roboflow-clip: {e}");
                        if let BackendError::MissingCredential(name) = &e {
                            eprintln!("set {name} to enable the identity gate or pass --dry-run.");
                        }
                        return ExitCode::from(2);
                    }
                };
                Some(Box::new(RoboflowClipAdapter::new(rf_client)))
            }
            "fal-clip-similarity" => {
                let fal_client = match FalClient::from_env(&a.cache) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("backend fal-clip-similarity: {e}");
                        if let BackendError::MissingCredential(name) = &e {
                            eprintln!("set {name} to enable the identity gate or pass --dry-run.");
                        }
                        return ExitCode::from(2);
                    }
                };
                Some(Box::new(FalClipSimilarityAdapter::new(fal_client)))
            }
            _ => unreachable!(),
        }
    };

    let mut req = InstructionEditRequest::new(concat_url, INSERT_INTO_SCENE_INSTRUCTION);
    req.seed = a.seed;

    let max_attempts = if a.strict_identity {
        INSERT_INTO_SCENE_MAX_RETRIES
    } else {
        1
    };
    let mut attempts: Vec<serde_json::Value> = Vec::new();
    let mut last_outcome = None;
    let mut last_identity = None;
    let mut accepted = false;

    for attempt in 0..max_attempts {
        if attempt > 0 {
            req.seed = Some(a.seed.unwrap_or(0).wrapping_add(attempt as u64));
        }
        let outcome = match kontext.instruction_edit(&req, mode) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("shot insert-into-scene: {e}");
                return ExitCode::from(2);
            }
        };

        let mut identity_payload: Option<serde_json::Value> = None;
        let mut identity_passed = true;
        if let Some(adapter) = identity_adapter.as_ref() {
            if outcome.response.image_bytes > 0 {
                let candidate_url = match image_arg_to_url(
                    outcome.response.image_path.to_str().unwrap_or(""),
                ) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("shot insert-into-scene: encode merged: {e}");
                        return ExitCode::from(2);
                    }
                };
                let id_req =
                    IdentityCheckRequest::new(product_url.clone(), candidate_url);
                match adapter.check(&id_req, a.threshold, mode) {
                    Ok(id_outcome) => {
                        identity_passed = id_outcome.response.passes_threshold;
                        identity_payload = Some(serde_json::json!({
                            "provider": id_outcome.provider,
                            "similarity": id_outcome.response.similarity,
                            "passes_threshold": id_outcome.response.passes_threshold,
                            "threshold": id_outcome.response.threshold,
                            "cost_estimate_usd": id_outcome.cost_estimate_usd,
                        }));
                        last_identity = identity_payload.clone();
                    }
                    Err(e) => {
                        if a.strict_identity {
                            eprintln!("shot insert-into-scene: identity gate: {e}");
                            return ExitCode::from(2);
                        }
                        eprintln!(
                            "shot insert-into-scene: identity gate skipped â {e}"
                        );
                        identity_payload = Some(serde_json::json!({
                            "skipped": true,
                            "reason": e.to_string(),
                        }));
                        last_identity = identity_payload.clone();
                    }
                }
            }
        }

        attempts.push(serde_json::json!({
            "attempt": attempt + 1,
            "seed": req.seed,
            "kontext": {
                "provider": outcome.provider,
                "request_hash": outcome.request_hash,
                "cached": outcome.cached,
                "cost_estimate_usd": outcome.cost_estimate_usd,
                "image_path": outcome.response.image_path,
                "width": outcome.response.width,
                "height": outcome.response.height,
            },
            "identity": identity_payload,
        }));

        last_outcome = Some(outcome);
        if identity_passed {
            accepted = true;
            break;
        }
        if !a.strict_identity {
            break;
        }
    }

    let outcome = match last_outcome {
        Some(o) => o,
        None => {
            eprintln!("shot insert-into-scene: no Kontext attempt ran");
            return ExitCode::from(2);
        }
    };

    if let Some(dest) = a.out.as_ref() {
        if outcome.response.image_bytes > 0 {
            if let Err(e) = std::fs::copy(&outcome.response.image_path, dest) {
                eprintln!(
                    "copy {} â {}: {e}",
                    outcome.response.image_path.display(),
                    dest.display()
                );
                return ExitCode::from(2);
            }
        }
    }

    let identity_warning = !accepted && last_identity.is_some();
    if identity_warning {
        eprintln!(
            "shot insert-into-scene: identity drift â similarity below threshold {:.2}",
            a.threshold
        );
    }

    let payload = serde_json::json!({
        "mode": outcome.mode,
        "provider": outcome.provider,
        "instruction": INSERT_INTO_SCENE_INSTRUCTION,
        "concat": {
            "path": concat_out.path,
            "width": concat_out.width,
            "height": concat_out.height,
            "left_width": concat_out.left_width,
            "right_width": concat_out.right_width,
        },
        "attempts": attempts,
        "accepted": accepted,
        "identity": last_identity,
        "result": outcome.response,
        "cost_estimate_usd": outcome.cost_estimate_usd,
        "strict_identity": a.strict_identity,
        "threshold": a.threshold,
    });
    let formatted = if a.pretty {
        serde_json::to_string_pretty(&payload)
    } else {
        serde_json::to_string(&payload)
    };
    println!(
        "{}",
        formatted.unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#))
    );

    if a.strict_identity && !accepted {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
