use audio_graph::ScheduledNode;
use dropseed_plugin_api::buffer::{AudioPortBuffer, AudioPortBufferMut, SharedBuffer};
use dropseed_plugin_api::ext::audio_ports::PluginAudioPortsExt;
use dropseed_plugin_api::ext::note_ports::PluginNotePortsExt;
use dropseed_plugin_api::{PluginInstanceID, ProcBuffers};
use fnv::FnvHashMap;
use smallvec::SmallVec;

use crate::plugin_host::event_io_buffers::{AutomationIoEvent, NoteIoEvent, PluginEventIoBuffers};
use crate::plugin_host::SharedPluginHostProcThread;
use crate::processor_schedule::tasks::{PluginTask, Task};

use super::super::super::error::GraphCompilerError;
use super::super::super::shared_pools::GraphSharedPools;
use super::super::super::{ChannelID, PortType};

pub(super) fn construct_activated_plugin_task(
    scheduled_node: &ScheduledNode,
    shared_pool: &GraphSharedPools,
    plugin_id: &PluginInstanceID,
    shared_processor: &SharedPluginHostProcThread,
    audio_ports_ext: &PluginAudioPortsExt,
    note_ports_ext: &PluginNotePortsExt,
    assigned_audio_buffers: FnvHashMap<ChannelID, (SharedBuffer<f32>, bool)>,
    assigned_note_buffers: FnvHashMap<ChannelID, (SharedBuffer<NoteIoEvent>, bool)>,
    assigned_automation_in_buffer: Option<(SharedBuffer<AutomationIoEvent>, bool)>,
    assigned_automation_out_buffer: Option<SharedBuffer<AutomationIoEvent>>,
) -> Result<Task, GraphCompilerError> {
    let mut audio_in: SmallVec<[AudioPortBuffer; 2]> = SmallVec::new();
    let mut audio_out: SmallVec<[AudioPortBufferMut; 2]> = SmallVec::new();
    let mut note_in_buffers: SmallVec<[SharedBuffer<NoteIoEvent>; 2]> = SmallVec::new();
    let mut note_out_buffers: SmallVec<[SharedBuffer<NoteIoEvent>; 2]> = SmallVec::new();
    let mut clear_audio_in_buffers: SmallVec<[SharedBuffer<f32>; 2]> = SmallVec::new();
    let mut clear_note_in_buffers: SmallVec<[SharedBuffer<NoteIoEvent>; 2]> = SmallVec::new();

    for in_port in audio_ports_ext.inputs.iter() {
        let mut buffers: SmallVec<[SharedBuffer<f32>; 2]> =
            SmallVec::with_capacity(usize::from(in_port.channels));
        for channel_i in 0..in_port.channels {
            let channel_id = ChannelID {
                stable_id: in_port.stable_id,
                port_type: PortType::Audio,
                is_input: true,
                channel: channel_i,
            };

            let buffer = assigned_audio_buffers.get(&channel_id).ok_or(
                GraphCompilerError::UnexpectedError(format!(
                    "Abstract schedule did not assign a buffer to every port in node {:?}",
                    scheduled_node
                )),
            )?;

            buffers.push(buffer.0.clone());

            if buffer.1 {
                clear_audio_in_buffers.push(buffer.0.clone());
            }
        }

        audio_in.push(AudioPortBuffer::_new(
            buffers,
            shared_pool.buffers.audio_buffer_pool.buffer_size() as u32,
        ));
        // TODO: assign proper latency information to AudioPortBuffers?
    }
    for out_port in audio_ports_ext.outputs.iter() {
        let mut buffers: SmallVec<[SharedBuffer<f32>; 2]> =
            SmallVec::with_capacity(usize::from(out_port.channels));
        for channel_i in 0..out_port.channels {
            let channel_id = ChannelID {
                stable_id: out_port.stable_id,
                port_type: PortType::Audio,
                is_input: false,
                channel: channel_i,
            };

            let buffer = assigned_audio_buffers.get(&channel_id).ok_or(
                GraphCompilerError::UnexpectedError(format!(
                    "Abstract schedule did not assign a buffer to every port in node {:?}",
                    scheduled_node
                )),
            )?;

            buffers.push(buffer.0.clone());
        }

        audio_out.push(AudioPortBufferMut::_new(
            buffers,
            shared_pool.buffers.audio_buffer_pool.buffer_size() as u32,
        ));
        // TODO: assign proper latency information to AudioPortBuffers?
    }

    for in_port in note_ports_ext.inputs.iter() {
        let channel_id = ChannelID {
            stable_id: in_port.stable_id,
            port_type: PortType::Note,
            is_input: true,
            channel: 0,
        };

        let buffer = assigned_note_buffers.get(&channel_id).ok_or(
            GraphCompilerError::UnexpectedError(format!(
                "Abstract schedule did not assign a buffer to every port in node {:?}",
                scheduled_node
            )),
        )?;

        note_in_buffers.push(buffer.0.clone());

        if buffer.1 {
            clear_note_in_buffers.push(buffer.0.clone());
        }
    }
    for out_port in note_ports_ext.outputs.iter() {
        let channel_id = ChannelID {
            stable_id: out_port.stable_id,
            port_type: PortType::Note,
            is_input: false,
            channel: 0,
        };

        let buffer = assigned_note_buffers.get(&channel_id).ok_or(
            GraphCompilerError::UnexpectedError(format!(
                "Abstract schedule did not assign a buffer to every port in node {:?}",
                scheduled_node
            )),
        )?;

        note_out_buffers.push(buffer.0.clone());
    }

    Ok(Task::Plugin(PluginTask {
        plugin_id: plugin_id.clone(),
        shared_processor: shared_processor.clone(),
        buffers: ProcBuffers { audio_in, audio_out },
        event_buffers: PluginEventIoBuffers {
            note_in_buffers,
            note_out_buffers,
            clear_note_in_buffers,
            automation_in_buffer: assigned_automation_in_buffer,
            automation_out_buffer: assigned_automation_out_buffer,
        },
        clear_audio_in_buffers,
    }))
}