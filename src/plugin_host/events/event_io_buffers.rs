use clack_host::events::io::EventBuffer;
use clack_host::utils::Cookie;
use dropseed_plugin_api::buffer::SharedBuffer;
use dropseed_plugin_api::ParamID;
use smallvec::SmallVec;

use crate::utils::reducing_queue::ReducFnvProducerRefMut;

use super::super::channel::ProcToMainParamValue;
use super::super::events::{
    sanitizer::PluginEventOutputSanitizer, NoteEvent, ParamEvent, PluginEvent,
};

// TODO: remove pubs
pub struct PluginEventIoBuffers {
    pub unmixed_param_in_buffers: Option<SmallVec<[SharedBuffer<ParamEvent>; 2]>>,
    /// Only for internal plugin (e.g. timeline or macros)
    pub param_out_buffer: Option<SharedBuffer<ParamEvent>>,

    // TODO: remove options
    pub unmixed_note_in_buffers: SmallVec<[Option<SmallVec<[SharedBuffer<NoteEvent>; 2]>>; 2]>,
    pub note_out_buffers: SmallVec<[Option<SharedBuffer<NoteEvent>>; 2]>,
}

impl PluginEventIoBuffers {
    pub fn clear_before_process(&mut self) {
        if let Some(buffer) = &mut self.param_out_buffer {
            buffer.truncate();
        }

        for buffer in self.note_out_buffers.iter().flatten() {
            buffer.truncate();
        }
    }

    pub fn write_input_events(&self, raw_event_buffer: &mut EventBuffer) -> (bool, bool) {
        let wrote_note_event = self.write_input_note_events(raw_event_buffer);
        let wrote_param_event = self.write_input_param_events(raw_event_buffer);

        (wrote_note_event, wrote_param_event)
    }

    fn write_input_note_events(&self, raw_event_buffer: &mut EventBuffer) -> bool {
        // TODO: make this clearer
        let in_events = self
            .unmixed_note_in_buffers
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.as_ref().map(|e| (i, e)))
            .flat_map(|(i, b)| b.iter().map(move |b| (i, b.borrow())));

        let mut wrote_note_event = false;

        for (note_port_index, buffer) in in_events {
            for event in buffer.iter() {
                let event = PluginEvent::NoteEvent {
                    note_port_index: note_port_index as i16,
                    event: *event,
                };
                event.write_to_clap_buffer(raw_event_buffer);
                wrote_note_event = true;
            }
        }

        wrote_note_event
    }

    fn write_input_param_events(&self, raw_event_buffer: &mut EventBuffer) -> bool {
        let mut wrote_param_event = false;
        for in_buf in self.unmixed_param_in_buffers.iter().flatten() {
            for event in in_buf.borrow().iter() {
                // TODO: handle cookies?
                let event = PluginEvent::ParamEvent { cookie: Cookie::empty(), event: *event };
                event.write_to_clap_buffer(raw_event_buffer);
                wrote_param_event = true;
            }
        }
        wrote_param_event
    }

    pub fn read_output_events(
        &mut self,
        raw_event_buffer: &EventBuffer,
        mut external_parameter_queue: Option<
            &mut ReducFnvProducerRefMut<ParamID, ProcToMainParamValue>,
        >,
        sanitizer: &mut PluginEventOutputSanitizer,
        param_target_plugin_id: u64,
    ) {
        let events_iter = raw_event_buffer
            .iter()
            .filter_map(|e| PluginEvent::read_from_clap(e, param_target_plugin_id));
        let events_iter = sanitizer.sanitize(events_iter);

        for event in events_iter {
            match event {
                PluginEvent::NoteEvent { note_port_index, event } => {
                    if let Some(Some(b)) = self.note_out_buffers.get(note_port_index as usize) {
                        b.borrow_mut().push(event)
                    }
                }
                PluginEvent::ParamEvent { cookie: _, event } => {
                    if let Some(buffer) = &mut self.param_out_buffer {
                        buffer.borrow_mut().push(event)
                    }

                    if let Some(queue) = external_parameter_queue.as_mut() {
                        if let Some(value) =
                            ProcToMainParamValue::from_param_event(event.event_type)
                        {
                            queue.set_or_update(ParamID::new(event.parameter_id), value);
                        }
                    }
                }
            }
        }
    }
}