//! Tauri managed state for the live session: the currently-open detected project
//! (M1) and the active build run (M3 - its cancel signal + live snapshot).

use crate::runner::ActiveRun;
use crate::unreal::{DetectedPlugin, DetectedProject};
use std::sync::{Mutex, RwLock};

#[derive(Default)]
pub struct AppState {
    pub current: RwLock<Option<DetectedProject>>,
    /// The currently-open plugin (when a `.uplugin`, not a `.uproject`, was opened).
    /// Powers the plugin Actions tab's package action. Mutually exclusive with
    /// `current` in practice - the gate opens one or the other.
    pub current_plugin: RwLock<Option<DetectedPlugin>>,
    /// The in-flight (or most recent) build **or plugin-package** run;
    /// `cancel_build`/`active_run` reach through it. Replaced when a new run starts.
    pub run: Mutex<Option<ActiveRun>>,
}
