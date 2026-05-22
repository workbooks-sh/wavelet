//! Tools the executor dispatches plan steps to. One module per
//! capability — each is responsible for its own external surface.

pub mod composite;
pub mod css_only;
pub mod omni_edit;
pub mod veo_regen;
