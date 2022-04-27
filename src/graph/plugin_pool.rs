use audio_graph::{DefaultPortType, Graph, NodeRef, PortRef};
use basedrop::Shared;
use basedrop::SharedCell;
use fnv::FnvHashMap;
use rusty_daw_core::SampleRate;
use smallvec::SmallVec;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{cell::UnsafeCell, hash::Hash};

use crate::host::{Host, HostInfo};
use crate::plugin::ext::audio_ports::AudioPortsExtension;
use crate::plugin::{PluginAudioThread, PluginMainThread, PluginSaveState};
use crate::plugin_scanner::PluginFormat;
use crate::ProcessStatus;

use super::schedule::delay_comp_node::DelayCompNode;
use super::PortID;

// TODO: Clean this up.

/// Used for debugging and verifying purposes.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PluginInstanceType {
    Internal,
    Clap,
    Sum,
    DelayComp,
    GraphInput,
    GraphOutput,
}

impl std::fmt::Debug for PluginInstanceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                PluginInstanceType::Internal => "Int",
                PluginInstanceType::Clap => "CLAP",
                PluginInstanceType::Sum => "Sum",
                PluginInstanceType::DelayComp => "Dly",
                PluginInstanceType::GraphInput => "GraphIn",
                PluginInstanceType::GraphOutput => "GraphOut",
            }
        )
    }
}

impl From<PluginFormat> for PluginInstanceType {
    fn from(p: PluginFormat) -> Self {
        match p {
            PluginFormat::Internal => PluginInstanceType::Internal,
            PluginFormat::Clap => PluginInstanceType::Clap,
        }
    }
}

/// Used to uniquely identify a plugin instance and for debugging purposes.
pub struct PluginInstanceID {
    pub(crate) node_id: NodeRef,
    pub(crate) format: PluginInstanceType,
    name: Option<Shared<String>>,
}

impl std::fmt::Debug for PluginInstanceID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let id: usize = self.node_id.into();
        match self.format {
            PluginInstanceType::Internal => {
                write!(f, "Int({})_{}", &**self.name.as_ref().unwrap(), id)
            }
            _ => {
                write!(f, "{:?}_{}", self.format, id)
            }
        }
    }
}

impl Clone for PluginInstanceID {
    fn clone(&self) -> Self {
        Self {
            node_id: self.node_id,
            format: self.format,
            name: self.name.as_ref().map(|n| Shared::clone(n)),
        }
    }
}

impl PartialEq for PluginInstanceID {
    fn eq(&self, other: &Self) -> bool {
        self.node_id.eq(&other.node_id)
    }
}

impl Eq for PluginInstanceID {}

impl Hash for PluginInstanceID {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.node_id.hash(state)
    }
}

/// Used to sync plugin state between the main thread and audio thread counterparts.
pub(crate) struct PluginInstanceChannel {
    pub restart_requested: AtomicBool,
    pub process_requested: AtomicBool,
    pub callback_requested: AtomicBool,

    active: AtomicBool,
    // TODO: parameter stuff
}

impl PluginInstanceChannel {
    pub fn new() -> Self {
        Self {
            restart_requested: AtomicBool::new(false),
            process_requested: AtomicBool::new(false),
            callback_requested: AtomicBool::new(false),
            active: AtomicBool::new(false),
        }
    }
}

pub(crate) struct PluginMainThreadInstance {
    pub plugin: Box<dyn PluginMainThread>,
    pub host_request: Host,

    id: PluginInstanceID,
}

pub(crate) struct PluginAudioThreadInstance {
    pub plugin: Option<UnsafeCell<Box<dyn PluginAudioThread>>>,
    pub host_request: Host,
    pub last_process_status: UnsafeCell<ProcessStatus>,
}

impl PluginAudioThreadInstance {
    pub fn deactivated_clone(&self) -> Self {
        PluginAudioThreadInstance {
            // The only time we are cloning this is when we want to deactivate/drop
            // the plugin's audio thread, so we don't care about keeping it.
            plugin: None,
            host_request: self.host_request.clone(),
            last_process_status: UnsafeCell::new(unsafe { *self.last_process_status.get() }),
        }
    }
}

// Safe because this only ever gets dereferenced once it is sent to the audio thread,
// and it stays on the audio thread for the rest of its lifetime. We need this in order
// to use basedrop's `SharedMut` persistent data structure.
//
// Also, `Send` is already a requirement for the `PluginAudioThread` trait, so we
// shouldn't have any issues if this gets sent to a different audio thread in a new
// schedule.
unsafe impl Send for PluginAudioThreadInstance {}
// Safe because this only ever gets dereferenced once it is sent to the audio thread,
// and it stays on the audio thread for the rest of its lifetime. We need this in order
// to use basedrop's `SharedMut` persistent data structure.
unsafe impl Sync for PluginAudioThreadInstance {}

#[derive(Clone)]
pub(crate) struct SharedPluginAudioThreadInstance {
    pub shared: Shared<SharedCell<PluginAudioThreadInstance>>,
    id: PluginInstanceID,
}

impl SharedPluginAudioThreadInstance {
    fn new(
        plugin: Option<Box<dyn PluginAudioThread>>,
        id: PluginInstanceID,
        host_request: Host,
        coll_handle: &basedrop::Handle,
    ) -> Self {
        Self {
            shared: Shared::new(
                &coll_handle,
                SharedCell::new(Shared::new(
                    &coll_handle,
                    PluginAudioThreadInstance {
                        plugin: plugin.map(|p| UnsafeCell::new(p)),
                        host_request,
                        last_process_status: UnsafeCell::new(ProcessStatus::Continue),
                    },
                )),
            ),
            id,
        }
    }

    pub fn id(&self) -> &PluginInstanceID {
        &self.id
    }
}

struct LoadedPluginInstance {
    main_thread: PluginMainThreadInstance,
    save_state: PluginSaveState,
    audio_ports_ext: AudioPortsExtension,
}

struct PluginInstance {
    loaded: Option<LoadedPluginInstance>,
    audio_thread: SharedPluginAudioThreadInstance,

    audio_in_channel_refs: Vec<PortRef>,
    audio_out_channel_refs: Vec<PortRef>,
    format: PluginFormat,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DelayCompKey {
    pub node_id: NodeRef,
    pub port_i: u16,
    pub delay: u32,
}

#[derive(Clone)]
pub(crate) struct SharedDelayCompNode {
    pub shared: Shared<UnsafeCell<DelayCompNode>>,
    pub active: bool,
}

impl SharedDelayCompNode {
    pub fn new(delay: u32, coll_handle: &basedrop::Handle) -> Self {
        Self {
            shared: Shared::new(coll_handle, UnsafeCell::new(DelayCompNode::new(delay))),
            active: true,
        }
    }

    pub fn delay(&self) -> u32 {
        // Safe because we are only borrowing this immutably.
        let delay_node = unsafe { &*self.shared.get() };
        delay_node.delay()
    }
}

pub(crate) struct PluginInstancePool {
    pub delay_comp_nodes: FnvHashMap<DelayCompKey, SharedDelayCompNode>,

    graph_plugins: Vec<Option<PluginInstance>>,
    free_graph_plugins: Vec<NodeRef>,

    host_info: Shared<HostInfo>,

    coll_handle: basedrop::Handle,

    num_plugins: usize,

    sample_rate: SampleRate,
    min_frames: usize,
    max_frames: usize,
}

impl PluginInstancePool {
    pub fn new(
        abstract_graph: &mut Graph<PluginInstanceID, PortID, DefaultPortType>,
        num_graph_in_audio_channels: u16,
        num_graph_out_audio_channels: u16,
        coll_handle: basedrop::Handle,
        host_info: Shared<HostInfo>,
        sample_rate: SampleRate,
        min_frames: usize,
        max_frames: usize,
    ) -> (Self, PluginInstanceID, PluginInstanceID) {
        let mut new_self = Self {
            delay_comp_nodes: FnvHashMap::default(),
            graph_plugins: Vec::new(),
            free_graph_plugins: Vec::new(),
            host_info,
            coll_handle,
            num_plugins: 0,
            sample_rate,
            min_frames,
            max_frames,
        };

        // --- Add the graph input node to the graph --------------------------

        let graph_in_node_id = if let Some(node_id) = new_self.free_graph_plugins.pop() {
            node_id
        } else {
            new_self.graph_plugins.push(None);
            NodeRef::new(new_self.graph_plugins.len())
        };

        let graph_in_id = PluginInstanceID {
            node_id: graph_in_node_id,
            format: PluginInstanceType::GraphInput,
            name: None,
        };

        let graph_in_node_ref = abstract_graph.node(graph_in_id.clone());
        // If this isn't right then I did something wrong.
        assert_eq!(graph_in_node_ref, graph_in_id.node_id);

        let mut graph_in_channel_refs: Vec<PortRef> =
            Vec::with_capacity(usize::from(num_graph_in_audio_channels));
        for i in 0..num_graph_in_audio_channels {
            let port_ref = abstract_graph
                .port(graph_in_node_ref, DefaultPortType::Audio, PortID::AudioOut(i))
                .unwrap();
            graph_in_channel_refs.push(port_ref);
        }

        let host_request = Host {
            info: Shared::clone(&new_self.host_info),
            plugin_channel: Shared::new(&new_self.coll_handle, PluginInstanceChannel::new()),
        };

        let node_i: usize = graph_in_id.node_id.into();
        new_self.graph_plugins[node_i] = Some(PluginInstance {
            loaded: None,
            audio_in_channel_refs: Vec::new(),
            audio_out_channel_refs: graph_in_channel_refs,
            format: PluginFormat::Internal,
            audio_thread: SharedPluginAudioThreadInstance::new(
                None,
                graph_in_id.clone(),
                host_request,
                &new_self.coll_handle,
            ),
        });

        new_self.num_plugins += 1;

        // --- Add the graph output node to the graph --------------------------

        let graph_out_node_id = if let Some(node_id) = new_self.free_graph_plugins.pop() {
            node_id
        } else {
            new_self.graph_plugins.push(None);
            NodeRef::new(new_self.graph_plugins.len())
        };

        let graph_out_id = PluginInstanceID {
            node_id: graph_out_node_id,
            format: PluginInstanceType::GraphOutput,
            name: None,
        };

        let graph_out_node_ref = abstract_graph.node(graph_out_id.clone());
        // If this isn't right then I did something wrong.
        assert_eq!(graph_out_node_ref, graph_out_id.node_id);

        let mut graph_out_channel_refs: Vec<PortRef> =
            Vec::with_capacity(usize::from(num_graph_out_audio_channels));
        for i in 0..num_graph_out_audio_channels {
            let port_ref = abstract_graph
                .port(graph_out_node_ref, DefaultPortType::Audio, PortID::AudioIn(i))
                .unwrap();
            graph_out_channel_refs.push(port_ref);
        }

        let host_request = Host {
            info: Shared::clone(&new_self.host_info),
            plugin_channel: Shared::new(&new_self.coll_handle, PluginInstanceChannel::new()),
        };

        let node_i: usize = graph_out_id.node_id.into();
        new_self.graph_plugins[node_i] = Some(PluginInstance {
            loaded: None,
            audio_in_channel_refs: graph_out_channel_refs,
            audio_out_channel_refs: Vec::new(),
            format: PluginFormat::Internal,
            audio_thread: SharedPluginAudioThreadInstance::new(
                None,
                graph_out_id.clone(),
                host_request,
                &new_self.coll_handle,
            ),
        });

        new_self.num_plugins += 1;

        (new_self, graph_in_id, graph_out_id)
    }

    pub fn add_graph_plugin(
        &mut self,
        plugin: Option<Box<dyn PluginMainThread>>,
        mut save_state: PluginSaveState,
        debug_name: Shared<String>,
        abstract_graph: &mut Graph<PluginInstanceID, PortID, DefaultPortType>,
        activate: bool,
    ) -> PluginInstanceID {
        let node_id = if let Some(node_id) = self.free_graph_plugins.pop() {
            node_id
        } else {
            self.graph_plugins.push(None);
            NodeRef::new(self.graph_plugins.len())
        };

        let id = PluginInstanceID {
            node_id,
            format: save_state.key.format.into(),
            name: Some(debug_name),
        };

        let node_ref = abstract_graph.node(id.clone());
        // If this isn't right then I did something wrong.
        assert_eq!(node_ref, id.node_id);

        let host_request = Host {
            info: Shared::clone(&self.host_info),
            plugin_channel: Shared::new(&self.coll_handle, PluginInstanceChannel::new()),
        };

        let (main_thread, audio_ports_ext) = if let Some(plugin) = plugin {
            let audio_ports_ext = plugin.audio_ports_extension(&host_request);
            let (num_audio_in, num_audio_out) = audio_ports_ext.total_in_out_channels();

            save_state.audio_in_out_channels = (num_audio_in as u16, num_audio_out as u16);

            (
                Some(PluginMainThreadInstance {
                    plugin,
                    host_request: host_request.clone(),
                    id: id.clone(),
                }),
                Some(audio_ports_ext),
            )
        } else {
            (None, None)
        };

        save_state.activated = false;

        let mut audio_in_channel_refs: Vec<PortRef> =
            Vec::with_capacity(usize::from(save_state.audio_in_out_channels.0));
        let mut audio_out_channel_refs: Vec<PortRef> =
            Vec::with_capacity(usize::from(save_state.audio_in_out_channels.1));
        let (audio_in_channels, audio_out_channels) = if let Some(audio_ports_ext) =
            &audio_ports_ext
        {
            let (audio_in_channels, audio_out_channels) = audio_ports_ext.total_in_out_channels();
            save_state.audio_in_out_channels =
                (audio_in_channels as u16, audio_out_channels as u16);
            (audio_in_channels as u16, audio_out_channels as u16)
        } else {
            // If the plugin failed to load, try to retrieve the number of channels
            // from the save state
            save_state.audio_in_out_channels
        };
        for i in 0..audio_in_channels {
            let port_ref =
                abstract_graph.port(node_id, DefaultPortType::Audio, PortID::AudioIn(i)).unwrap();
            audio_in_channel_refs.push(port_ref);
        }
        for i in 0..audio_out_channels {
            let port_ref =
                abstract_graph.port(node_id, DefaultPortType::Audio, PortID::AudioOut(i)).unwrap();
            audio_out_channel_refs.push(port_ref);
        }

        let format = save_state.key.format;

        let audio_thread =
            SharedPluginAudioThreadInstance::new(None, id.clone(), host_request, &self.coll_handle);

        let new_instance = if main_thread.is_some() {
            PluginInstance {
                loaded: Some(LoadedPluginInstance {
                    main_thread: main_thread.unwrap(),
                    save_state,
                    audio_ports_ext: audio_ports_ext.unwrap(),
                }),
                audio_in_channel_refs,
                audio_out_channel_refs,
                format,
                audio_thread,
            }
        } else {
            PluginInstance {
                loaded: None,
                audio_in_channel_refs,
                audio_out_channel_refs,
                format,
                audio_thread,
            }
        };
        let node_i: usize = node_id.into();
        self.graph_plugins[node_i] = Some(new_instance);

        self.num_plugins += 1;

        log::debug!("Added plugin instance {:?} to audio graph", &id);

        if activate {
            self.activate_plugin_instance(&id, abstract_graph, false);
        }

        id
    }

    pub fn remove_graph_plugin(
        &mut self,
        id: &PluginInstanceID,
        abstract_graph: &mut Graph<PluginInstanceID, PortID, DefaultPortType>,
    ) {
        // Deactivate the plugin first.
        self.deactivate_plugin_instance(id);

        let node_i: usize = id.node_id.into();
        if let Some(plugin_instance) = self.graph_plugins[node_i].take() {
            // Drop the plugin instance here.
            let _ = plugin_instance;

            // Re-use this node ID for the next new plugin.
            self.free_graph_plugins.push(id.node_id);

            self.num_plugins -= 1;

            abstract_graph.delete_node(id.node_id).unwrap();

            log::debug!("Removed plugin instance {:?} from audio graph", id);
        } else {
            log::warn!("Could not remove plugin instance {:?} from audio graph: Plugin was not found in the graph", id);
        }
    }

    pub fn activate_plugin_instance(
        &mut self,
        id: &PluginInstanceID,
        abstract_graph: &mut Graph<PluginInstanceID, PortID, DefaultPortType>,
        check_for_port_change: bool,
    ) -> bool {
        let node_i: usize = id.node_id.into();
        if let Some(plugin_instance) = &mut self.graph_plugins[node_i] {
            if let Some(loaded_plugin) = &mut plugin_instance.loaded {
                if loaded_plugin
                    .main_thread
                    .host_request
                    .plugin_channel
                    .active
                    .load(Ordering::Relaxed)
                {
                    log::warn!("Tried to activate plugin that is already active");

                    // Cannot activate plugin that is already active.
                    return false;
                }

                let recompile_audio_graph = if check_for_port_change {
                    let new_audio_ports_ext = loaded_plugin
                        .main_thread
                        .plugin
                        .audio_ports_extension(&loaded_plugin.main_thread.host_request);

                    if new_audio_ports_ext != loaded_plugin.audio_ports_ext {
                        loaded_plugin.audio_ports_ext = new_audio_ports_ext;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };

                if recompile_audio_graph {
                    // Make sure the abstract graph has the updated number of ports.

                    let node_id = loaded_plugin.main_thread.id.node_id;

                    let (audio_in_channels, audio_out_channels) =
                        loaded_plugin.audio_ports_ext.total_in_out_channels();

                    if audio_in_channels > plugin_instance.audio_in_channel_refs.len() {
                        let len = plugin_instance.audio_in_channel_refs.len() as u16;
                        for i in len..audio_in_channels as u16 {
                            let port_ref = abstract_graph
                                .port(node_id, DefaultPortType::Audio, PortID::AudioIn(i))
                                .unwrap();
                            plugin_instance.audio_in_channel_refs.push(port_ref);
                        }
                    } else if audio_in_channels < plugin_instance.audio_in_channel_refs.len() {
                        let n_to_remove =
                            plugin_instance.audio_in_channel_refs.len() - audio_in_channels;
                        for _ in 0..n_to_remove {
                            let port_ref = plugin_instance.audio_in_channel_refs.pop().unwrap();
                            if let Err(e) = abstract_graph.delete_port(port_ref) {
                                log::error!(
                                    "Unexpected error while removing port from abstract graph: {}",
                                    e
                                );
                            }
                        }
                    }

                    if audio_out_channels > plugin_instance.audio_out_channel_refs.len() {
                        let len = plugin_instance.audio_in_channel_refs.len() as u16;
                        for i in len..audio_out_channels as u16 {
                            let port_ref = abstract_graph
                                .port(node_id, DefaultPortType::Audio, PortID::AudioOut(i))
                                .unwrap();
                            plugin_instance.audio_out_channel_refs.push(port_ref);
                        }
                    } else if audio_out_channels < plugin_instance.audio_out_channel_refs.len() {
                        let n_to_remove =
                            plugin_instance.audio_out_channel_refs.len() - audio_out_channels;
                        for _ in 0..n_to_remove {
                            let port_ref = plugin_instance.audio_out_channel_refs.pop().unwrap();
                            if let Err(e) = abstract_graph.delete_port(port_ref) {
                                log::error!(
                                    "Unexpected error while removing port from abstract graph: {}",
                                    e
                                );
                            }
                        }
                    }
                }

                match loaded_plugin.main_thread.plugin.activate(
                    self.sample_rate,
                    self.min_frames,
                    self.max_frames,
                    &loaded_plugin.main_thread.host_request,
                    &self.coll_handle,
                ) {
                    Ok(plugin_audio_thread) => {
                        let mut new_audio_thread =
                            (*plugin_instance.audio_thread.shared.get()).deactivated_clone();
                        new_audio_thread.plugin = Some(UnsafeCell::new(plugin_audio_thread));

                        if recompile_audio_graph {
                            // If recompiling the audio graph because the plugin changed its audio
                            // port configuration, we don't send the plugin's audio thread to the
                            // current schedule (or else bad things will happen). Instead we create
                            // a new shared pointer that will be added to the next schedule that
                            // gets compiled.
                            plugin_instance.audio_thread.shared = Shared::new(
                                &self.coll_handle,
                                SharedCell::new(Shared::new(&self.coll_handle, new_audio_thread)),
                            );
                        } else {
                            // If the plugin did not change its audio port configuration, then it's
                            // safe to replace the plugin's audio thread in the current schedule.
                            plugin_instance
                                .audio_thread
                                .shared
                                .set(Shared::new(&self.coll_handle, new_audio_thread));
                        }

                        loaded_plugin
                            .main_thread
                            .host_request
                            .plugin_channel
                            .active
                            .store(true, Ordering::Relaxed);
                        loaded_plugin.save_state.activated = true;

                        log::trace!("Successfully activated plugin instance {:?}", &id);

                        return recompile_audio_graph;
                    }
                    Err(e) => {
                        log::error!(
                            "Error while activating plugin instance {:?}: {}",
                            loaded_plugin.main_thread.id,
                            e
                        );
                    }
                }
            }
        }

        false
    }

    pub fn deactivate_plugin_instance(&mut self, id: &PluginInstanceID) {
        let node_i: usize = id.node_id.into();
        if let Some(plugin_instance) = &mut self.graph_plugins[node_i] {
            if let Some(loaded_plugin) = &mut plugin_instance.loaded {
                if !loaded_plugin
                    .main_thread
                    .host_request
                    .plugin_channel
                    .active
                    .load(Ordering::Relaxed)
                {
                    // Plugin is already inactive.
                    return;
                }

                loaded_plugin
                    .main_thread
                    .plugin
                    .deactivate(&loaded_plugin.main_thread.host_request);

                loaded_plugin
                    .main_thread
                    .host_request
                    .plugin_channel
                    .active
                    .store(false, Ordering::Relaxed);

                loaded_plugin.save_state.activated = false;
            }

            // Overwrite the current audio thread task with a blank one. This will
            // cause the audio thread part to be dropped on the next process cycle.
            let mut new_audio_thread =
                (*plugin_instance.audio_thread.shared.get()).deactivated_clone();
            new_audio_thread.plugin = None; // This should already be `None`, just a sanity check.
            plugin_instance
                .audio_thread
                .shared
                .set(Shared::new(&self.coll_handle, new_audio_thread));
        }
    }

    #[inline]
    pub fn is_plugin_loaded(&self, id: &PluginInstanceID) -> Result<bool, ()> {
        let node_i: usize = id.node_id.into();
        if let Some(plugin_instance) = &self.graph_plugins[node_i] {
            Ok(plugin_instance.loaded.is_some())
        } else {
            Err(())
        }
    }

    #[inline]
    pub fn is_plugin_active(&self, id: &PluginInstanceID) -> Result<bool, ()> {
        let node_i: usize = id.node_id.into();
        if let Some(plugin_instance) = &self.graph_plugins[node_i] {
            if let Some(loaded_plugin) = plugin_instance.loaded.as_ref() {
                Ok(loaded_plugin
                    .main_thread
                    .host_request
                    .plugin_channel
                    .active
                    .load(Ordering::Relaxed))
            } else {
                Ok(false)
            }
        } else {
            Err(())
        }
    }

    #[inline]
    pub fn get_audio_ports_ext(
        &self,
        id: &PluginInstanceID,
    ) -> Result<Option<&AudioPortsExtension>, ()> {
        let node_i: usize = id.node_id.into();
        if let Some(plugin_instance) = &self.graph_plugins[node_i] {
            if let Some(loaded) = &plugin_instance.loaded {
                Ok(Some(&loaded.audio_ports_ext))
            } else {
                Ok(None)
            }
        } else {
            Err(())
        }
    }

    #[inline]
    pub fn get_audio_in_channel_refs(&self, id: &PluginInstanceID) -> Result<&[PortRef], ()> {
        let node_i: usize = id.node_id.into();
        if let Some(plugin_instance) = &self.graph_plugins[node_i] {
            Ok(&plugin_instance.audio_in_channel_refs)
        } else {
            Err(())
        }
    }

    #[inline]
    pub fn get_audio_out_channel_refs(&self, id: &PluginInstanceID) -> Result<&[PortRef], ()> {
        let node_i: usize = id.node_id.into();
        if let Some(plugin_instance) = &self.graph_plugins[node_i] {
            Ok(&plugin_instance.audio_out_channel_refs)
        } else {
            Err(())
        }
    }

    #[inline]
    pub fn get_plugin_format(&self, id: &PluginInstanceID) -> Result<PluginFormat, ()> {
        let node_i: usize = id.node_id.into();
        if let Some(plugin_instance) = &self.graph_plugins[node_i] {
            Ok(plugin_instance.format)
        } else {
            Err(())
        }
    }

    pub fn num_plugins(&self) -> usize {
        self.num_plugins
    }

    #[inline]
    pub fn get_graph_plugin_audio_thread(
        &self,
        id: &PluginInstanceID,
    ) -> Result<&SharedPluginAudioThreadInstance, ()> {
        let node_i: usize = id.node_id.into();
        if let Some(plugin_instance) = self.graph_plugins[node_i].as_ref() {
            Ok(&plugin_instance.audio_thread)
        } else {
            Err(())
        }
    }

    pub fn iter_plugin_ids(&self) -> impl Iterator<Item = NodeRef> + '_ {
        self.graph_plugins
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.as_ref().map(|_| NodeRef::new(i)))
    }

    pub fn get_graph_plugin_save_state(&self, node_id: NodeRef) -> Result<&PluginSaveState, ()> {
        let node_i: usize = node_id.into();
        if let Some(plugin) = self.graph_plugins[node_i].as_ref() {
            if let Some(loaded) = &plugin.loaded {
                Ok(&loaded.save_state)
            } else {
                Err(())
            }
        } else {
            Err(())
        }
    }

    pub fn host_info(&self) -> &Shared<HostInfo> {
        &self.host_info
    }

    pub fn update_main_thread(
        &mut self,
        abstract_graph: &mut Graph<PluginInstanceID, PortID, DefaultPortType>,
    ) -> bool {
        let mut recompile_audio_graph = false;
        let mut plugins_to_restart: SmallVec<[PluginInstanceID; 8]> = SmallVec::new();

        // TODO: Find a more optimal way to poll for requests? We can't just use an spsc message
        // channel because CLAP plugins use the same host pointer for requests in the audio thread
        // and in the main thread. Is there some thread-safe way to get a list of only the plugins
        // that have requested something?
        for plugin in self.graph_plugins.iter_mut().filter_map(|p| p.as_mut()) {
            if let Some(loaded_plugin) = plugin.loaded.as_mut() {
                if loaded_plugin
                    .main_thread
                    .host_request
                    .plugin_channel
                    .callback_requested
                    .load(Ordering::Relaxed)
                {
                    loaded_plugin
                        .main_thread
                        .host_request
                        .plugin_channel
                        .callback_requested
                        .store(false, Ordering::Relaxed);

                    loaded_plugin
                        .main_thread
                        .plugin
                        .on_main_thread(&loaded_plugin.main_thread.host_request);
                }
                if loaded_plugin
                    .main_thread
                    .host_request
                    .plugin_channel
                    .process_requested
                    .load(Ordering::Relaxed)
                {
                    loaded_plugin
                        .main_thread
                        .host_request
                        .plugin_channel
                        .process_requested
                        .store(false, Ordering::Relaxed);

                    // TODO
                }
                if loaded_plugin
                    .main_thread
                    .host_request
                    .plugin_channel
                    .restart_requested
                    .load(Ordering::Relaxed)
                {
                    loaded_plugin
                        .main_thread
                        .host_request
                        .plugin_channel
                        .restart_requested
                        .store(false, Ordering::Relaxed);

                    plugins_to_restart.push(loaded_plugin.main_thread.id.clone());
                }
            }
        }

        for id in plugins_to_restart.iter() {
            self.deactivate_plugin_instance(&id);
            if self.activate_plugin_instance(&id, abstract_graph, true) {
                recompile_audio_graph = true;
            }
        }

        recompile_audio_graph
    }
}