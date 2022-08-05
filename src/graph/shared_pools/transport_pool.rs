use atomic_refcell::{AtomicRefCell, AtomicRefMut};
use basedrop::Shared;

use crate::schedule::tasks::TransportTask;

#[derive(Clone)]
pub struct SharedTransportTask {
    shared: Shared<AtomicRefCell<TransportTask>>,
}

impl SharedTransportTask {
    pub fn new(t: TransportTask, coll_handle: &basedrop::Handle) -> Self {
        Self { shared: Shared::new(coll_handle, AtomicRefCell::new(t)) }
    }

    pub fn borrow_mut<'a>(&'a self) -> AtomicRefMut<'a, TransportTask> {
        self.shared.borrow_mut()
    }
}

pub(crate) struct TransportPool {
    // TODO: Add the ability to have more than one tranport.
    pub transport: SharedTransportTask,
}