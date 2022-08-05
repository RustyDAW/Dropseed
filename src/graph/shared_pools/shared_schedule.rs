use atomic_refcell::AtomicRefCell;
use basedrop::{Shared, SharedCell};

use crate::schedule::Schedule;
use crate::utils::thread_id::SharedThreadIDs;

// Required so we can send the schedule from the main thread to the process
// thread.
//
// This is safe because the schedule is only ever dereferenced in the process
// thread. The only reason why the main thread holds onto these shared
// pointers of buffers and `PluginAudioThread`s is so it can construct new
// schedules with them. The main thread never dereferences these pointers.
unsafe impl Send for Schedule {}
// Required so we can send the schedule from the main thread to the process
// thread. The fact that the main thread holds onto shared pointers of
// buffers and `PluginAudioThread`s requires this to be `Sync` as well.
//
// This is safe because the schedule is only ever dereferenced in the process
// thread. The only reason why the main thread holds onto these shared
// pointers of buffers and `PluginAudioThread`s is so it can construct new
// schedules with them. The main thread never dereferences these pointers.
unsafe impl Sync for Schedule {}

pub(crate) struct SharedSchedule {
    schedule: Shared<SharedCell<AtomicRefCell<Schedule>>>,
    thread_ids: SharedThreadIDs,
    coll_handle: basedrop::Handle,
}

// Implement Debug so we can send it in an event.
impl std::fmt::Debug for SharedSchedule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SharedSchedule")
    }
}

impl SharedSchedule {
    pub fn new(
        schedule: Schedule,
        thread_ids: SharedThreadIDs,
        coll_handle: &basedrop::Handle,
    ) -> (Self, Self) {
        let schedule = Shared::new(
            coll_handle,
            SharedCell::new(Shared::new(coll_handle, AtomicRefCell::new(schedule))),
        );

        (
            Self {
                schedule: schedule.clone(),
                thread_ids: thread_ids.clone(),
                coll_handle: coll_handle.clone(),
            },
            Self { schedule, thread_ids, coll_handle: coll_handle.clone() },
        )
    }

    pub fn set_new_schedule(&mut self, schedule: Schedule, coll_handle: &basedrop::Handle) {
        self.schedule.set(Shared::new(coll_handle, AtomicRefCell::new(schedule)));
    }

    pub fn process_interleaved(
        &mut self,
        audio_in: &[f32],
        audio_in_channels: usize,
        audio_out: &mut [f32],
        audio_out_channels: usize,
    ) {
        let latest_schedule = self.schedule.get();

        let mut schedule = latest_schedule.borrow_mut();

        if let Some(audio_thread_id) = self.thread_ids.external_audio_thread_id() {
            if std::thread::current().id() != audio_thread_id {
                self.thread_ids
                    .set_external_audio_thread_id(std::thread::current().id(), &self.coll_handle);
            }
        } else {
            self.thread_ids
                .set_external_audio_thread_id(std::thread::current().id(), &self.coll_handle);
        }

        schedule.process_interleaved(audio_in, audio_in_channels, audio_out, audio_out_channels);
    }
}