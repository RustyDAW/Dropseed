use std::path::PathBuf;

use crate::{
    engine::EngineActivatedInfo,
    graph::{
        AudioGraphRestoredInfo, AudioGraphSaveState, GraphCompilerError, NewPluginRes,
        PluginActivatedInfo, PluginActivationError, PluginEdgesChangedInfo, PluginInstanceID,
    },
    plugin_scanner::RescanPluginDirectoriesRes,
    AudioPortsExtension,
};

#[derive(Debug)]
#[non_exhaustive]
pub enum DAWEngineEvent {
    /// Sent whenever the engine is deactivated.
    ///
    /// If the result is `Ok(save_state)`, then it means that the engine
    /// deactivated gracefully via calling `RustyDAWEngine::deactivate_engine()`,
    /// and the latest save state of the audio graph is returned.
    ///
    /// If the result is `Err(e)`, then it means that the engine deactivated
    /// because of a unrecoverable audio graph compiler error.
    ///
    /// To keep using the audio graph, you must reactivate the engine with
    /// `RustyDAWEngine::activate_engine()`, and then restore the audio graph
    /// from an existing save state if you wish using
    /// `RustyDAWEngine::restore_audio_graph_from_save_state()`.
    EngineDeactivated(Result<AudioGraphSaveState, GraphCompilerError>),

    /// This message is sent whenever the engine successfully activates.
    EngineActivated(EngineActivatedInfo),

    /// When this message is received, it means that the audio graph is starting
    /// the process of restoring from a save state.
    ///
    /// Reset your UI as if you are loading up a project for the first time, and
    /// wait for the `PluginInstancesAdded` and `PluginEdgesChanged` events in
    /// order to repopulate the UI.
    ///
    /// If the audio graph is in an invalid state as a result of restoring from
    /// the save state, then the `EngineDeactivatedBecauseGraphIsInvalid` event
    /// will be sent instead.
    AudioGraphCleared,

    /// When this message is received, it means that the audio graph has finished
    /// the process of restoring from a save state.
    AudioGraphRestoredFromSaveState(AudioGraphRestoredInfo),

    /// This message is sent after the user requests the latest save state from
    /// calling `RustyDAWEngine::request_latest_save_state()`.
    ///
    /// Use the latest save state as a backup in case a plugin crashes or a bug
    /// in the audio graph compiler causes the audio graph to be in an invalid
    /// state, resulting in the audio engine stopping.
    NewSaveState(AudioGraphSaveState),

    /// This message is sent whenever new plugins are added to the audio graph.
    PluginInstancesAdded(Vec<NewPluginRes>),

    /// This message is sent whenever plugins are removed from the audio graph.
    ///
    /// Note that the host will always send a `PluginEdgesChanged` event
    /// before this event if any of the removed plugins had connected
    /// edges. This `PluginEdgesChanged` event will have all edges that
    /// were connected to any of the removed plugins removed.
    PluginInstancesRemoved(Vec<PluginInstanceID>),

    /// This message is sent whenever the edges (connections of ports) of
    /// plugins change (including when adding/removing plugins).
    PluginEdgesChanged(PluginEdgesChangedInfo),

    /// Sent whenever a plugin becomes deactivated. When a plugin is deactivated
    /// you cannot access any of its methods until it is reactivated.
    PluginDeactivated {
        plugin_id: PluginInstanceID,
        /// If this is `Ok(())`, then it means the plugin was gracefully
        /// deactivated from user request.
        ///
        /// If this is `Err(e)`, then it means the plugin became deactivated
        /// because it failed to restart.
        status: Result<(), PluginActivationError>,
    },

    /// Sent whenever a plugin becomes activated after being deactivated or
    /// when the plugin restarts.
    ///
    /// Make sure your UI updates the port configuration on this plugin.
    PluginActivated {
        plugin_id: PluginInstanceID,
        /// If this is `Some(audio_ports)`, then it means that the plugin has
        /// updated its audio port configuration.
        ///
        /// If this is `None`, then it means that the plugin has not changed
        /// its audio port configuration since the last time it was activated.
        new_audio_ports: Option<AudioPortsExtension>,
    },

    PluginScanner(PluginScannerEvent),
    // TODO: More stuff
}

#[derive(Debug)]
#[non_exhaustive]
pub enum PluginScannerEvent {
    #[cfg(feature = "clap-host")]
    /// A new CLAP plugin scan path was added.
    ClapScanPathAdded(PathBuf),
    #[cfg(feature = "clap-host")]
    /// A CLAP plugin scan path was removed.
    ClapScanPathRemoved(PathBuf),

    /// A request to rescan all plugin directories has finished. Update
    /// the list of available plugins in your UI.
    RescanFinished(RescanPluginDirectoriesRes),
}

impl From<PluginScannerEvent> for DAWEngineEvent {
    fn from(e: PluginScannerEvent) -> Self {
        DAWEngineEvent::PluginScanner(e)
    }
}
