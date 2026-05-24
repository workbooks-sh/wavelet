use std::process::ExitCode;
use crate::handlers::image_depth_map::handle_image_depth_map;
use crate::handlers::image_ocr::run_image_ocr;
use crate::cli_args::ImageOp;
use crate::handlers::image_bg_remove::handle_image_bg_remove;
use crate::handlers::image_composite::handle_image_composite;
use crate::handlers::image_contrast::handle_image_contrast;
use crate::handlers::image_identity_check::handle_image_identity_check;
use crate::handlers::image_isolate::handle_image_isolate;
use crate::handlers::image_scrim::handle_image_scrim;
use crate::handlers::image_verify_shot::handle_image_verify_shot;
use crate::handlers::util::emit_analysis;

/// Top-level `wavelet image <op>` dispatch.
pub fn run_image(op: ImageOp) -> ExitCode {
    use crate::image_analysis;
    match op {
        ImageOp::NegativeSpace {
            image,
            rows,
            cols,
            use_depth,
            pretty,
        } => emit_analysis(pretty, || {
            image_analysis::negative_space::analyze_with_depth(&image, rows, cols, use_depth)
        }),
        ImageOp::DepthMap { image, out, pretty } => {
            handle_image_depth_map(image, out, pretty)
        }
        ImageOp::Saliency {
            image,
            rows,
            cols,
            top_n,
            pretty,
        } => emit_analysis(pretty, || image_analysis::saliency::analyze(&image, rows, cols, top_n)),
        ImageOp::Ocr {
            image,
            backend,
            dry_run,
            max_cost,
            cache,
            pretty,
        } => run_image_ocr(image, backend, dry_run, max_cost, cache, pretty),
        ImageOp::Contrast {
            image,
            region,
            text_color,
            threshold,
            pretty,
        } => handle_image_contrast(&image, &region, &text_color, threshold, pretty),
        ImageOp::Scrim {
            image,
            rows,
            cols,
            threshold,
            out,
            pretty,
        } => handle_image_scrim(&image, rows, cols, threshold, out.as_deref(), pretty),
        ImageOp::Composite {
            foreground,
            background,
            out,
            scale,
            y_offset,
        } => handle_image_composite(&foreground, &background, &out, scale, y_offset),
        ImageOp::Isolate {
            image,
            prompt,
            backend,
            dry_run,
            max_cost,
            cache,
            out,
            pretty,
        } => handle_image_isolate(
            image,
            prompt,
            &backend,
            dry_run,
            max_cost,
            &cache,
            out.as_deref(),
            pretty,
        ),
        ImageOp::IdentityCheck {
            reference,
            candidate,
            threshold,
            backend,
            dry_run,
            max_cost,
            cache,
            pretty,
        } => handle_image_identity_check(
            reference,
            candidate,
            threshold,
            &backend,
            dry_run,
            max_cost,
            &cache,
            pretty,
        ),
        ImageOp::BgRemove {
            image,
            backend,
            dry_run,
            max_cost,
            cache,
            out,
            pretty,
        } => handle_image_bg_remove(image, &backend, dry_run, max_cost, &cache, out.as_deref(), pretty),
        ImageOp::VerifyShot {
            image,
            criteria,
            backend,
            dry_run,
            max_cost,
            cache,
            pretty,
        } => handle_image_verify_shot(image, criteria, &backend, dry_run, max_cost, &cache, pretty),
    }
}
