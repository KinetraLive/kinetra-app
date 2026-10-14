//! The local Control API (`docs/CONTROL_API.md`): the WebSocket and REST interface other
//! programs on this computer or the LAN use to control Kinetra (Bitfocus Companion, scripts).
//!
//! Its state is remote control's tree in the local flavor (`remote::state::Flavor::Local`):
//! layer keys `<screen>.<layer>`, the logical on-air state (take → clear), and what a control
//! surface shows besides (staged changes, readiness, running cues, sources, Automation off).

#[cfg(test)]
mod tests;

use serde_json::Value;

use crate::core::remote::state::{self, Flavor};
use crate::core::state::AppState;

impl AppState {
    /// The Control API's state tree, built from memory (the asset list is read from disk only
    /// after a media change). Call from a thread that may block briefly.
    pub fn control_api_tree(&self) -> Value {
        let mut input = self.remote_build_input();
        input.flavor = Flavor::Local;
        input.take_locked = false;
        state::build(self, &input)
    }
}
