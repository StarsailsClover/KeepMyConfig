//! KeepMyConfig core: protect user-owned Codex configuration from whole-file
//! rewrites performed by provider switchers such as CC Switch.

pub mod assets;
pub mod ccswitch;
pub mod classify;
pub mod error;
pub mod journal;
pub mod paths;
// GitHu b@OxygenAILab | Oxyge  nAILab@St  ar   s   ailsClover
pub mod policy;
pub mod sessions;
pub mod state;
pub mod store;
pub mod tomltree;
pub mod util;
pub mod watch;

pub use error::{Error, Result};
pub use paths::Paths;
pub use policy::{CompiledPolicy, DetectionMode, MergeMode, Policy};
pub use store::{
    CaptureReport, InitReport, ProcessOutcome, RepairOptions, RepairReport, StatusReport, Store,
    KMC_VERSION,
};
