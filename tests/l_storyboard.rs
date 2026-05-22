//! Integration test for the L-Storyboard structured-attribute schema.
//!
//! Loads `fixtures/l-storyboard-example.json` and asserts:
//! 1. Every shot deserializes with `attributes: Some(_)`.
//! 2. Every `ShotAttributes::validate()` passes.
//! 3. `shot_prompt_fragment` returns the L-Storyboard assembly
//!    (i.e. `attributes.to_prompt()`) and ignores the legacy
//!    `Generation`-payload path.
//! 4. The fixture round-trips through serde unchanged.

use wavelet::storyboard::{shot_prompt_fragment, Storyboard};

const FIXTURE: &str = include_str!("fixtures/l-storyboard-example.json");

#[test]
fn fixture_parses_with_attributes_on_every_shot() {
    let sb: Storyboard = serde_json::from_str(FIXTURE).expect("parse fixture");
    assert_eq!(sb.shots.len(), 4);
    for s in &sb.shots {
        assert!(
            s.attributes.is_some(),
            "shot {} is missing attributes",
            s.id
        );
    }
}

#[test]
fn fixture_attributes_validate() {
    let sb: Storyboard = serde_json::from_str(FIXTURE).unwrap();
    for s in &sb.shots {
        s.attributes.as_ref().unwrap().validate().unwrap_or_else(|e| {
            panic!("shot {} failed validate: {e}", s.id);
        });
    }
}

#[test]
fn fixture_prompt_fragment_uses_l_storyboard_assembly() {
    let sb: Storyboard = serde_json::from_str(FIXTURE).unwrap();
    let hero = &sb.shots[1];
    let attrs_prompt = hero.attributes.as_ref().unwrap().to_prompt();
    let frag = shot_prompt_fragment(hero);
    assert_eq!(frag, attrs_prompt);
    assert!(frag.contains("1968 Porsche 911 GT3"));
    assert!(frag.contains("anamorphic"));
    assert!(frag.contains("backlit by rising sun"));
    assert!(!frag.contains("ignored"));
}

#[test]
fn fixture_round_trips_through_serde() {
    let sb: Storyboard = serde_json::from_str(FIXTURE).unwrap();
    let json = serde_json::to_string(&sb).unwrap();
    let back: Storyboard = serde_json::from_str(&json).unwrap();
    assert_eq!(back.shots.len(), sb.shots.len());
    for (a, b) in sb.shots.iter().zip(back.shots.iter()) {
        assert_eq!(a.attributes, b.attributes);
    }
}

#[test]
fn fixture_hero_prompt_snapshot() {
    let sb: Storyboard = serde_json::from_str(FIXTURE).unwrap();
    let frag = shot_prompt_fragment(&sb.shots[1]);
    assert_eq!(
        frag,
        "a 1968 Porsche 911 GT3 in racing yellow. \
         idles, engine off, parked at pit lane. \
         on wet asphalt as the sun crests the ridge. \
         Shot: WS 50mm, low angle, 3/4 front, \
         anamorphic, shallow DoF, slight chromatic fringe. \
         Lighting: backlit by rising sun, mist-diffused. \
         Style: cinematic, A24-flavored, restrained color."
    );
}
