//! WEEP's run configurations, free of gpui: Fleet's `.fleet/run.json` as
//! data ([`RunConfig`], [`RunFile`]), the command a configuration resolves
//! to ([`Command`]: `command` rows run their program, `cargo` rows run
//! `cargo` with their `cargoArgs`), and a running process whose output
//! arrives line by line ([`Run`]); debugging a `cargo` row builds it with
//! cargo's JSON messages and picks the executable from its artifacts
//! ([`cargo`], M8). Fleet's field names are the vocabulary
//! (`documentation.yaml` of the run plugin: `name`, `workingDir`,
//! `environment`, `environmentFile`, `dependsOn`, `allowParallelRun`,
//! `program`, `args`).

pub mod cargo;
pub mod config;
pub mod process;

pub use config::{Command, RunConfig, RunFile};
pub use process::{Event, Run};
