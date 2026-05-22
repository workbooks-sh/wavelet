use std::process::ExitCode;
use crate::handlers::util::ShotStillVariantArgs;
use crate::handlers::util::format_winner_reason;
use crate::handlers::util::image_arg_to_url;

/// (auto-generated placeholder)
pub fn run_shot_still_variants(a: ShotStillVariantArgs) -> ExitCode {
    use crate::backends::fal::{FalClient, FalFluxSchnellAdapter, FalVisionVerifyAdapter};
    use crate::backends::image::{
        Txt2ImgBackend, Txt2ImgRequest, VisionVerifyBackend, VisionVerifyRequest,
    };
    use crate::backends::{BackendError, RunMode};
    use crate::variants::{
        check_aggregate_cost, default_criteria, enumerate_seeds, estimate_line, select_winner,
        CostGate, SelectPolicy, VariantManifest, VariantRecord,
    };

    let ShotStillVariantArgs {
        req,
        backend,
        variants,
        select_raw,
        criteria,
        max_variants_cost,
        mode,
        cache,
        out,
        pretty,
        dry_run,
    } = a;

    let policy = match SelectPolicy::parse(&select_raw) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(3);
        }
    };
    if backend != "fal-flux-schnell" {
        eprintln!("unknown --backend '{backend}', want fal-flux-schnell");
        return ExitCode::from(3);
    }
    let client_factory = || -> Result<FalClient, BackendError> {
        if dry_run {
            Ok(FalClient::with_key("dry-run", &cache))
        } else {
            FalClient::from_env(&cache)
        }
    };
    let adapter = match client_factory() {
        Ok(c) => FalFluxSchnellAdapter::new(c),
        Err(e) => {
            eprintln!("backend fal-flux-schnell: {e}");
            if let BackendError::MissingCredential(name) = &e {
                eprintln!("set {name} or pass --dry-run to preview.");
            }
            return ExitCode::from(2);
        }
    };
    let per_call_cost = adapter.estimate_cost(&req).cost_usd;
    eprintln!("{}", estimate_line(variants, per_call_cost));
    if let CostGate::Block {
        estimated_usd,
        ceiling_usd,
    } = check_aggregate_cost(per_call_cost, variants, max_variants_cost)
    {
        eprintln!(
            "aggregate cost ${estimated_usd:.4} exceeds --max-variants-cost ${ceiling_usd:.4}"
        );
        return ExitCode::from(2);
    }
    let seeds = enumerate_seeds(req.seed, variants);
    let adapter_ref = &adapter;
    let req_ref = &req;
    let outcomes: Vec<(u64, Result<_, BackendError>, u64)> = std::thread::scope(|s| {
        let handles: Vec<_> = seeds
            .iter()
            .copied()
            .map(|seed_value| {
                s.spawn(move || {
                    let mut per_req: Txt2ImgRequest = req_ref.clone();
                    per_req.seed = Some(seed_value);
                    let start = std::time::Instant::now();
                    let r = adapter_ref.generate(&per_req, mode);
                    (seed_value, r, start.elapsed().as_millis() as u64)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let need_vlm = matches!(policy, SelectPolicy::MaxVlm) && !dry_run;
    let criteria_resolved = if criteria.is_empty() {
        default_criteria(None)
    } else {
        criteria
    };
    let verifier = if need_vlm {
        client_factory().ok().map(FalVisionVerifyAdapter::new)
    } else {
        None
    };

    let mut records: Vec<VariantRecord<crate::backends::image::ImageResult>> = Vec::new();
    let mut succeeded = 0u32;
    let mut total_cost = 0.0f32;
    for (idx, (seed_value, result, elapsed_ms)) in outcomes.into_iter().enumerate() {
        match result {
            Ok(outcome) => {
                succeeded += 1;
                total_cost += outcome.cost_estimate_usd;
                let mut rec = VariantRecord {
                    index: idx as u32,
                    seed: seed_value,
                    response: Some(outcome.response.clone()),
                    provider: Some(outcome.provider.clone()),
                    request_hash: Some(outcome.request_hash.clone()),
                    cached: outcome.cached,
                    cost_estimate_usd: outcome.cost_estimate_usd,
                    elapsed_ms,
                    vlm_pass_count: None,
                    vlm_total: None,
                    error: None,
                };
                if let Some(v) = verifier.as_ref() {
                    if let Ok(url) =
                        image_arg_to_url(&outcome.response.image_path.to_string_lossy())
                    {
                        let vreq = VisionVerifyRequest::new(url, criteria_resolved.clone());
                        if let Ok(vo) = v.verify(&vreq, RunMode::Live { max_cost_usd: 0.05 }) {
                            let pass = vo
                                .response
                                .findings
                                .iter()
                                .filter(|f| {
                                    matches!(
                                        f.status,
                                        crate::backends::image::FindingStatus::Pass
                                    )
                                })
                                .count() as u32;
                            rec.vlm_pass_count = Some(pass);
                            rec.vlm_total = Some(vo.response.findings.len() as u32);
                            total_cost += vo.cost_estimate_usd;
                        }
                    }
                }
                records.push(rec);
            }
            Err(e) => {
                records.push(VariantRecord {
                    index: idx as u32,
                    seed: seed_value,
                    response: None,
                    provider: Some("fal-flux-schnell".into()),
                    request_hash: None,
                    cached: false,
                    cost_estimate_usd: 0.0,
                    elapsed_ms,
                    vlm_pass_count: None,
                    vlm_total: None,
                    error: Some(e.to_string()),
                });
            }
        }
    }
    let bracket_history: Vec<crate::variants::BracketRound> = Vec::new();
    if matches!(policy, SelectPolicy::PairwiseTournament) {
        eprintln!(
            "pairwise-tournament not wired for shot-still; falling back to first-success"
        );
    }
    let winner_policy = if matches!(policy, SelectPolicy::PairwiseTournament) {
        SelectPolicy::First
    } else {
        policy
    };
    let winner = select_winner(winner_policy, &records);
    let winner_reason = format_winner_reason(winner_policy, winner, &records);
    if let (Some(idx), Some(dest)) = (winner, out.as_ref()) {
        if let Some(r) = records.get(idx as usize) {
            if let Some(resp) = &r.response {
                if resp.image_bytes > 0 {
                    if let Err(e) = std::fs::copy(&resp.image_path, dest) {
                        eprintln!(
                            "copy {} â {}: {e}",
                            resp.image_path.display(),
                            dest.display()
                        );
                        return ExitCode::from(2);
                    }
                }
            }
        }
    }
    let manifest = VariantManifest {
        select: policy,
        requested: variants,
        succeeded,
        total_cost_usd: total_cost,
        variants: records,
        winner,
        winner_reason,
        bracket: bracket_history,
    };
    let formatted = if pretty {
        serde_json::to_string_pretty(&manifest)
    } else {
        serde_json::to_string(&manifest)
    };
    println!(
        "{}",
        formatted.unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#))
    );
    if succeeded == 0 {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}
