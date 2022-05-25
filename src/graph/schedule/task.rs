use smallvec::SmallVec;

use crate::graph::shared_pool::{SharedBuffer, SharedDelayCompNode, SharedPluginHostAudioThread};
use crate::plugin::process_info::ProcBuffers;
use crate::ProcInfo;

use super::sum::SumTask;

pub(crate) enum Task {
    Plugin(PluginTask),
    DelayComp(DelayCompTask),
    Sum(SumTask),
    DeactivatedPlugin(DeactivatedPluginTask),
}

impl std::fmt::Debug for Task {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Task::Plugin(t) => {
                let mut f = f.debug_struct("Plugin");

                f.field("id", &t.plugin.id());

                if !t.buffers.audio_in.is_empty() {
                    let mut s = String::new();
                    for b in t.buffers.audio_in.iter() {
                        s.push_str(&format!("{:?}, ", b))
                    }

                    f.field("audio_in", &s);
                }

                if !t.buffers.audio_out.is_empty() {
                    let mut s = String::new();
                    for b in t.buffers.audio_out.iter() {
                        s.push_str(&format!("{:?}, ", b))
                    }

                    f.field("audio_out", &s);
                }

                f.finish()
            }
            Task::DelayComp(t) => {
                let mut f = f.debug_struct("DelayComp");

                f.field("audio_in", t.audio_in.id());
                f.field("audio_out", t.audio_out.id());
                f.field("delay", &t.delay_comp_node.delay());

                f.finish()
            }
            Task::Sum(t) => {
                let mut f = f.debug_struct("Sum");

                let mut s = String::new();
                for b in t.audio_in.iter() {
                    s.push_str(&format!("{:?}, ", b.id()))
                }
                f.field("audio_in", &s);

                f.field("audio_out", &format!("{:?}", t.audio_out.id()));

                f.finish()
            }
            Task::DeactivatedPlugin(t) => {
                let mut f = f.debug_struct("DeactivatedPlugin");

                let mut s = String::new();
                for (b_in, b_out) in t.audio_through.iter() {
                    s.push_str(&format!("(in: {:?}, out: {:?})", b_in.id(), b_out.id()));
                }
                f.field("audio_through", &s);

                let mut s = String::new();
                for b in t.extra_audio_out.iter() {
                    s.push_str(&format!("{:?}, ", b.id()))
                }
                f.field("extra_audio_out", &s);

                f.finish()
            }
        }
    }
}

impl Task {
    pub fn process(&mut self, proc_info: &ProcInfo) {
        match self {
            Task::Plugin(task) => task.process(proc_info),
            Task::DelayComp(task) => task.process(proc_info),
            Task::Sum(task) => task.process(proc_info),
            Task::DeactivatedPlugin(task) => task.process(proc_info),
        }
    }
}

pub(crate) struct PluginTask {
    pub plugin: SharedPluginHostAudioThread,

    pub buffers: ProcBuffers,
}

impl PluginTask {
    fn process(&mut self, proc_info: &ProcInfo) {
        // SAFETY
        // - This is only ever borrowed here in this method in the audio thread.
        // - The schedule verifier has ensured that a single plugin instance does
        // not appear twice within the same schedule, so no data races can occur.
        let plugin_audio_thread = unsafe { &mut *self.plugin.plugin.get() };

        plugin_audio_thread.process(proc_info, &mut self.buffers);
    }
}

pub(crate) struct DelayCompTask {
    pub delay_comp_node: SharedDelayCompNode,

    pub audio_in: SharedBuffer<f32>,
    pub audio_out: SharedBuffer<f32>,
}

impl DelayCompTask {
    fn process(&mut self, proc_info: &ProcInfo) {
        // SAFETY
        // - This is only ever borrowed here in this method in the audio thread.
        // - The schedule verifier has ensured that a single node instance does
        // not appear twice within the same schedule, so no data races can occur.
        let delay_comp_node = unsafe { &mut *self.delay_comp_node.node.get() };

        delay_comp_node.process(proc_info, &self.audio_in, &self.audio_out);
    }
}

pub(crate) struct DeactivatedPluginTask {
    pub audio_through: SmallVec<[(SharedBuffer<f32>, SharedBuffer<f32>); 4]>,
    pub extra_audio_out: SmallVec<[SharedBuffer<f32>; 4]>,
}

impl DeactivatedPluginTask {
    fn process(&mut self, proc_info: &ProcInfo) {
        // SAFETY
        // - These buffers are only ever borrowed in the audio thread.
        // - The schedule verifier has ensured that no data races can occur between parallel
        // audio threads due to aliasing buffer pointers.
        // - `proc_info.frames` will always be less than or equal to the allocated size of
        // all process audio buffers.
        unsafe {
            // Pass audio through the main ports.
            for (in_buf, out_buf) in self.audio_through.iter() {
                let in_buf = in_buf.slice_from_frames_unchecked(proc_info.frames);
                let out_buf = out_buf.slice_from_frames_unchecked_mut(proc_info.frames);

                out_buf.copy_from_slice(in_buf);
            }

            // Make sure any extra output buffers are cleared.
            for out_buf in self.extra_audio_out.iter() {
                let out_buf = out_buf.slice_from_frames_unchecked_mut(proc_info.frames);

                out_buf.fill(0.0);
            }
        }
    }
}
