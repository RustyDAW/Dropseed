use audio_graph::ScheduledNode;
use std::error::Error;

use dropseed_plugin_api::{buffer::DebugBufferID, PluginInstanceID};

use crate::engine::request::EdgeReq;
use crate::processor_schedule::ProcessorSchedule;

use super::{PortChannelID, PortType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectEdgeErrorType {
    SrcPluginDoesNotExist,
    DstPluginDoesNotExist,
    SrcPortDoesNotExist,
    DstPortDoesNotExist,
    Cycle,
    Unkown,
}

#[derive(Debug, Clone)]
pub struct ConnectEdgeError {
    pub error_type: ConnectEdgeErrorType,
    pub edge: EdgeReq,
}

impl Error for ConnectEdgeError {}

impl std::fmt::Display for ConnectEdgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.error_type {
            ConnectEdgeErrorType::SrcPluginDoesNotExist => {
                write!(
                    f,
                    "Could not add edge {:?} to graph: Source plugin does not exist",
                    &self.edge
                )
            }
            ConnectEdgeErrorType::DstPluginDoesNotExist => {
                write!(
                    f,
                    "Could not add edge {:?} to graph: Destination plugin does not exist",
                    &self.edge
                )
            }
            ConnectEdgeErrorType::SrcPortDoesNotExist => {
                write!(
                    f,
                    "Could not add edge {:?} to graph: Source port does not exist",
                    &self.edge
                )
            }
            ConnectEdgeErrorType::DstPortDoesNotExist => {
                write!(
                    f,
                    "Could not add edge {:?} to graph: Destination port does not exist",
                    &self.edge
                )
            }
            ConnectEdgeErrorType::Cycle => {
                write!(f, "Could not add edge {:?} to graph: Cycle detected", &self.edge)
            }
            ConnectEdgeErrorType::Unkown => {
                write!(f, "Could not add edge {:?} to graph: Unkown error", &self.edge)
            }
        }
    }
}

#[derive(Debug)]
pub enum GraphCompilerError {
    VerifierError(
        VerifyScheduleError,
        Vec<ScheduledNode<PluginInstanceID, PortChannelID, PortType>>,
        ProcessorSchedule,
    ),
    UnexpectedError(String, Vec<ScheduledNode<PluginInstanceID, PortChannelID, PortType>>),
}

impl Error for GraphCompilerError {}

impl std::fmt::Display for GraphCompilerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            GraphCompilerError::VerifierError(e, abstract_schedule, schedule) => {
                write!(f, "Failed to compile audio graph: {}\n\nOutput of abstract graph compiler: {:?}\n\nOutput of final compiler: {:?}", e, &abstract_schedule, &schedule)
            }
            GraphCompilerError::UnexpectedError(e, abstract_schedule) => {
                write!(f, "Failed to compile audio graph: Unexpected error: {}\n\nOutput of abstract graph compiler: {:?}", e, &abstract_schedule)
            }
        }
    }
}

#[allow(unused)]
#[derive(Debug, Clone)]
pub enum VerifyScheduleError {
    BufferAppearsTwiceInSameTask {
        buffer_id: DebugBufferID,
        task_info: String,
    },
    BufferAppearsTwiceInParallelTasks {
        buffer_id: DebugBufferID,
    },
    PluginInstanceAppearsTwiceInSchedule {
        plugin_id: PluginInstanceID,
    },
    /// This could be made just a warning and not an error, but it's still not what
    /// we want to happen.
    SumNodeWithLessThanTwoInputs {
        num_inputs: usize,
        task_info: String,
    },
}

impl Error for VerifyScheduleError {}

impl std::fmt::Display for VerifyScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            VerifyScheduleError::BufferAppearsTwiceInSameTask { buffer_id, task_info } => {
                write!(f, "Error detected in compiled audio graph: The buffer with ID {:?} appears more than once within the same task {}", buffer_id, task_info)
            }
            VerifyScheduleError::BufferAppearsTwiceInParallelTasks { buffer_id } => {
                write!(f, "Error detected in compiled audio graph: The buffer with ID {:?} appears more than once between the parallel tasks", buffer_id)
            }
            VerifyScheduleError::PluginInstanceAppearsTwiceInSchedule { plugin_id } => {
                write!(f, "Error detected in compiled audio graph: The plugin instance with ID {:?} appears more than once in the schedule", plugin_id)
            }
            VerifyScheduleError::SumNodeWithLessThanTwoInputs { num_inputs, task_info } => {
                write!(f, "Error detected in compiled audio graph: A Sum node was created with {} inputs in the task {}", num_inputs, task_info)
            }
        }
    }
}