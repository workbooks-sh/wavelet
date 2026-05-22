use std::process::ExitCode;

/// (auto-generated placeholder)
pub fn run_query_shader(shader: &str, frame: &std::path::Path, params_json: &str) -> ExitCode {
    use crate::agent::plan::validators::shader::{run_named_shader, KNOWN_SHADERS};

    let params: serde_json::Value = match serde_json::from_str(params_json) {
        Ok(v) => v,
        Err(e) => {
            let err = serde_json::json!({
                "error": "bad_params_json",
                "reason": e.to_string(),
            });
            println!("{}", serde_json::to_string(&err).unwrap());
            return ExitCode::from(2);
        }
    };

    if !KNOWN_SHADERS.contains(&shader) {
        let err = serde_json::json!({
            "error": "unknown_shader",
            "shader": shader,
            "available": KNOWN_SHADERS,
        });
        println!("{}", serde_json::to_string(&err).unwrap());
        return ExitCode::from(2);
    }

    match run_named_shader(shader, frame, &params) {
        Ok(res) => {
            println!("{}", serde_json::to_string(&res.to_json()).unwrap());
            if res.pass {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(e) => {
            let err = serde_json::json!({
                "error": "dispatch_failed",
                "shader": shader,
                "reason": e.to_string(),
            });
            println!("{}", serde_json::to_string(&err).unwrap());
            ExitCode::from(2)
        }
    }
}
