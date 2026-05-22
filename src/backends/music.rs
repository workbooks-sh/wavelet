//! Music-generation clusters — two shapes share the "music" category:
//!
//! - **`RefConditionedMusicGen`** (this trait) — text prompt +
//!   optional audio reference + duration + BPM. Members: MusicGen,
//!   Stable Audio. Hosted on Fal/Replicate. Open-weight, cheap.
//! - **`StructuredMusicGen`** (future trait) — text with section
//!   markers (`[intro 60bpm] [drop 130bpm]`). Members: Suno, Udio.
//!   Proprietary, higher quality.
//!
//! The two clusters share the same `MusicResult` shape so downstream
//! tools (audio mixer, velocity validator) consume them uniformly.

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use crate::velocity::VelocityProfile;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Cluster identifier for `RefConditionedMusicGen` — used in cache keys.
pub const CLUSTER_REF_COND: &str = "music_ref_conditioned";

/// One reference-conditioned music-gen request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefConditionedMusicRequest {
    /// Text prompt describing the desired music ("cinematic ambient
    /// strings building to a drop").
    pub prompt: String,
    /// Desired duration in seconds. Models clamp to their max length;
    /// MusicGen tops out at 30s per call.
    pub duration_secs: f32,
    /// Optional BPM target. Encoded into the prompt for MusicGen
    /// (which respects "120 bpm" in free text) and as a structured
    /// field for Stable Audio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f32>,
    /// Optional reference audio path (melody/style condition). Models
    /// that don't support audio conditioning ignore this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_audio: Option<String>,
    /// Optional model variant override (e.g. `"stereo-large"` for
    /// MusicGen). When `None`, the adapter picks a sensible default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_variant: Option<String>,
    /// Random seed for reproducibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl RefConditionedMusicRequest {
    /// Build a minimum-viable request.
    pub fn new(prompt: impl Into<String>, duration_secs: f32) -> Self {
        Self {
            prompt: prompt.into(),
            duration_secs,
            bpm: None,
            reference_audio: None,
            model_variant: None,
            seed: None,
        }
    }

    /// Convenience: realize a velocity profile into a prompt + duration.
    /// The prompt embeds the BPM curve as a sequence of section markers
    /// the way MusicGen-shaped models prefer (free-text BPM hints).
    pub fn from_velocity(
        velocity: &VelocityProfile,
        style: impl Into<String>,
    ) -> Self {
        let mean = velocity.mean_bpm;
        let style = style.into();
        let arc = render_velocity_arc(velocity);
        let prompt = format!("{style}, {arc}, {mean:.0} bpm average");
        Self {
            prompt,
            duration_secs: velocity.duration_secs,
            bpm: Some(mean),
            reference_audio: None,
            model_variant: None,
            seed: None,
        }
    }
}

/// Compact textual description of a velocity arc. Picks up to 4 key
/// anchors and emits "60bpm calm → 95bpm build → 130bpm drop → 70bpm
/// landing" so the prompt nudges the gen toward the agent-authored
/// pacing.
fn render_velocity_arc(velocity: &VelocityProfile) -> String {
    if velocity.anchors.is_empty() {
        return String::new();
    }
    // Pick representative anchors: first, last, and the two most
    // extreme-BPM anchors in between.
    let mut anchors = velocity.anchors.clone();
    let last_idx = anchors.len() - 1;
    let mut picked: Vec<usize> = vec![0, last_idx];
    if anchors.len() > 4 {
        // Sort interior anchors by |bpm − mean|, take top 2.
        let mean = velocity.mean_bpm;
        let mut interior: Vec<usize> = (1..last_idx).collect();
        interior.sort_by(|a, b| {
            (anchors[*b].bpm - mean)
                .abs()
                .partial_cmp(&(anchors[*a].bpm - mean).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for i in interior.into_iter().take(2) {
            picked.push(i);
        }
        picked.sort();
        picked.dedup();
    } else {
        for i in 1..last_idx {
            picked.push(i);
        }
        picked.sort();
        picked.dedup();
    }

    // Stable iteration order.
    let pieces: Vec<String> = picked
        .into_iter()
        .map(|i| {
            let a = &mut anchors[i];
            let label = a.label.take().unwrap_or_else(|| descriptor_for(a.bpm));
            format!("{:.0}bpm {}", a.bpm, label)
        })
        .collect();
    pieces.join(" → ")
}

fn descriptor_for(bpm: f32) -> String {
    match bpm as i32 {
        ..=70 => "calm".into(),
        71..=95 => "steady".into(),
        96..=115 => "build".into(),
        116..=140 => "drop".into(),
        _ => "frenetic".into(),
    }
}

/// Result of a music-gen synthesis. Shared with the future
/// `StructuredMusicGen` cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MusicResult {
    /// Provider identifier (`fal-musicgen`, `fal-stable-audio`).
    pub provider: String,
    /// Cached audio path on disk.
    pub audio_path: PathBuf,
    /// Audio file size in bytes.
    pub audio_bytes: u64,
    /// Mime type of the container.
    pub mime: String,
    /// Model variant actually used (e.g. `"stereo-large"`).
    pub model_variant: String,
    /// Prompt actually sent (after any adapter-side rewriting).
    pub prompt_sent: String,
    /// Duration the backend reports producing, in seconds. Some
    /// providers honor the request, some clamp; this is the ground
    /// truth.
    pub duration_secs: f32,
}

/// Cluster trait shared by every reference-conditioned music-gen
/// adapter.
pub trait RefConditionedMusicGenBackend {
    /// Provider name (`"fal-musicgen"`, …).
    fn name(&self) -> &'static str;

    /// Estimate the cost. Most providers in this cluster charge per
    /// second of generated audio.
    fn estimate_cost(&self, request: &RefConditionedMusicRequest) -> CostEstimate;

    /// Synthesize music. The cached audio file path is in the
    /// returned `MusicResult.audio_path`.
    fn generate(
        &self,
        request: &RefConditionedMusicRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<MusicResult>, BackendError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::velocity::Anchor;

    #[test]
    fn velocity_arc_describes_the_curve() {
        let v = VelocityProfile {
            duration_secs: 30.0,
            mean_bpm: 0.0,
            anchors: vec![
                Anchor { t: 0.0, bpm: 60.0, label: Some("calm".into()) },
                Anchor { t: 8.0, bpm: 95.0, label: Some("build".into()) },
                Anchor { t: 18.0, bpm: 130.0, label: Some("drop".into()) },
                Anchor { t: 30.0, bpm: 70.0, label: Some("landing".into()) },
            ],
        };
        let arc = render_velocity_arc(&v);
        assert!(arc.contains("60bpm"));
        assert!(arc.contains("130bpm"));
        assert!(arc.contains("→"));
    }

    #[test]
    fn from_velocity_emits_useful_prompt() {
        let v = VelocityProfile {
            duration_secs: 20.0,
            mean_bpm: 0.0,
            anchors: vec![
                Anchor { t: 0.0, bpm: 80.0, label: None },
                Anchor { t: 20.0, bpm: 120.0, label: None },
            ],
        };
        let req = RefConditionedMusicRequest::from_velocity(&v, "cinematic strings");
        assert!(req.prompt.contains("cinematic strings"));
        assert!(req.prompt.contains("bpm"));
        assert!((req.duration_secs - 20.0).abs() < 1e-6);
        assert!(req.bpm.is_some());
    }

    #[test]
    fn request_round_trips_through_json() {
        let req = RefConditionedMusicRequest {
            prompt: "ambient strings, 90 bpm".into(),
            duration_secs: 8.0,
            bpm: Some(90.0),
            reference_audio: None,
            model_variant: Some("stereo-large".into()),
            seed: Some(42),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: RefConditionedMusicRequest = serde_json::from_str(&json).unwrap();
        assert!((back.duration_secs - 8.0).abs() < 1e-6);
        assert_eq!(back.bpm, Some(90.0));
        assert_eq!(back.model_variant.as_deref(), Some("stereo-large"));
    }
}
