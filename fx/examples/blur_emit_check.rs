use wavelet_fx::{compile, src};
fn main() {
    let comp = src(0).blur(8.0).output();
    let out = compile(&comp).expect("compile");
    println!("=== pre_effects ===");
    for p in &out.passes[0].pre_effects {
        println!("  {:?}", p);
    }
}
