//! End-to-end demo: build a WaveletFx composition with Animato tweens, compile
//! it, and walk the resulting EmitOutput to show what a consumer (wavelet)
//! receives. Includes a frame-loop fragment that samples each tween at
//! several timecodes — exactly the call pattern the renderer will use.
//!
//! Run with: `cargo run --example animato_demo`

use wavelet_fx::{compile, noise, src, Easing, Tween, UniformKind};

fn main() {
    // Two animated parameters using the same `Tween` type wavelet uses for DOM
    // animation. No "shader time" concept — Animato is the timeline.
    let mod_depth = Tween::new(0.0_f32, 0.2)
        .duration(2.0)
        .easing(Easing::EaseInOutSine)
        .build();

    let warmth = Tween::new(0.9_f32, 1.2)
        .duration(4.0)
        .easing(Easing::EaseOutQuad)
        .build();

    let comp = src(0)
        .modulate(noise(4.0, 0.0), mod_depth)
        .color(warmth, 1.0, 1.0, 1.0)
        .output();

    let out = compile(&comp).expect("compile");

    println!("== EmitOutput ==");
    println!("passes:   {}", out.passes.len());
    println!("inputs:   {:?}", out.passes[0].inputs);
    println!("uniforms: {}", out.uniforms.len());
    for u in &out.uniforms {
        let kind = match &u.kind {
            UniformKind::Time => "Time".to_string(),
            UniformKind::Resolution => "Resolution".to_string(),
            UniformKind::Tween(_) => "Tween (Animato)".to_string(),
            other => format!("{:?}", other),
        };
        println!("  {:<14} :: {}", u.name, kind);
    }

    println!();
    println!("== Frame loop (consumer pattern) ==");
    println!("Sampling each Animato tween at fixed timecodes — same call");
    println!("pattern wavelet uses for DOM/CSS animation. Tween values would");
    println!("be written into the corresponding uniform buffer slots.");
    println!();

    let timecodes = [0.0_f32, 0.5, 1.0, 2.0, 4.0];
    for &t in &timecodes {
        print!("  t = {:>4.2}s :", t);
        for u in &out.uniforms {
            if matches!(u.kind, UniformKind::Tween(_)) {
                let v = u.sample_at(t).unwrap();
                print!("  {} = {:>5.3}", u.name, v);
            }
        }
        println!();
    }

    println!();
    println!("== Emitted WGSL ==");
    println!("{}", out.passes[0].wgsl);
}
