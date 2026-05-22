//! One-off helper — print every detected onset in an audio file as JSON.
//! `cargo run --release --example dump_onsets -- <audio.mp3>`

use wavelet::audio::DecodedAudio;
use wavelet::query::beat::detect_onsets_interleaved;
use std::env;
use std::path::PathBuf;

fn main() {
    let path: PathBuf = env::args().nth(1).expect("usage: dump_onsets <audio>").into();
    let audio = DecodedAudio::decode(&path).expect("decode");
    let onsets = detect_onsets_interleaved(&audio.samples, audio.sample_rate);
    println!("{{");
    println!("  \"file\": \"{}\",", path.display());
    println!("  \"sample_rate\": {},", audio.sample_rate);
    println!("  \"onset_count\": {},", onsets.len());
    println!("  \"onsets_ms\": [");
    for (i, ms) in onsets.iter().enumerate() {
        let comma = if i + 1 == onsets.len() { "" } else { "," };
        println!("    {ms}{comma}");
    }
    println!("  ]");
    println!("}}");
}
