mod clap_plugin_host;

pub mod audio_buffer;
pub mod c_char_helpers;
pub mod channel_map;
pub mod engine;
pub mod error;
pub mod host;
pub mod plugin;
pub mod process;
pub mod schedule;

#[derive(Debug, Clone, Copy, PartialEq)]
enum EngineState {
    Stopped,
    Running,
    Stopping,
}

pub struct RustyDAWEngine {
    state: EngineState,
}

impl RustyDAWEngine {}
