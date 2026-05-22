use wavelet_fx::{compile, parse, UniformKind};
fn main() {
    let src = r#"
        src(0).blend(src(1), prop("progress")).out
    "#;
    let comp = parse(src).expect("parse");
    let out = compile(&comp).expect("compile");
    println!("=== uniforms ({}) ===", out.uniforms.len());
    for u in &out.uniforms {
        println!("  {} :: {:?}", u.name, u.kind);
    }
    println!();
    println!("=== textures ===");
    for p in &out.passes[0].inputs {
        println!("  {:?}", p);
    }
    println!();
    println!("=== WGSL ===");
    println!("{}", out.passes[0].wgsl);
}
