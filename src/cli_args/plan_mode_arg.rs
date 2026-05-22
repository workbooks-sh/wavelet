//! PlanModeArg — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



/// Plan substrate engagement level surfaced via clap. Mirrors
/// `crate::agent::PlanMode`; kept separate so the CLI surface owns its
/// own `ValueEnum` impl without leaking clap into the agent crate.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum PlanModeArg {
    Off,
    Shadow,
    On,
}

impl From<PlanModeArg> for crate::agent::PlanMode {
    fn from(value: PlanModeArg) -> Self {
        match value {
            PlanModeArg::Off => crate::agent::PlanMode::Off,
            PlanModeArg::Shadow => crate::agent::PlanMode::Shadow,
            PlanModeArg::On => crate::agent::PlanMode::On,
        }
    }
}
