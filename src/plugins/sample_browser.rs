use basedrop::{Owned, Shared};
use clack_host::events::event_types::ParamValueEvent;
use meadowlark_core_types::{
    ParamF32, ParamF32Handle, SampleRate, Unit, DEFAULT_DB_GRADIENT, DEFAULT_SMOOTH_SECS,
};
use rtrb::{Consumer, Producer, RingBuffer};
use serde::{Deserialize, Serialize};

use crate::plugin::ext::params::{default_db_value_to_text, parse_text_to_f64};
use crate::plugin::{
    ext, PluginActivatedInfo, PluginAudioThread, PluginDescriptor, PluginFactory, PluginMainThread,
    PluginPreset,
};
use crate::resource_loader::PcmResource;
use crate::{
    EventBuffer, HostRequest, ParamID, ParamInfoFlags, PluginInstanceID, ProcBuffers, ProcInfo,
    ProcessStatus,
};

pub static SAMPLE_BROWSER_PLUG_RDN: &str = "app.meadowlark.sample-browser";

const MSG_BUFFER_SIZE: usize = 16;

pub struct SampleBrowserPlugFactory;

impl PluginFactory for SampleBrowserPlugFactory {
    fn description(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: SAMPLE_BROWSER_PLUG_RDN.into(),
            version: "0.1".into(),
            name: "Sample Browser".into(),
            vendor: "Meadowlark".into(),
            description: String::new(),
            url: String::new(),
            manual_url: String::new(),
            support_url: String::new(),
            features: String::new(),
        }
    }

    fn new(
        &mut self,
        host_request: HostRequest,
        _plugin_id: PluginInstanceID,
        _coll_handle: &basedrop::Handle,
    ) -> Result<Box<dyn PluginMainThread>, String> {
        Ok(Box::new(SampleBrowserPlugMainThread::new(host_request)))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct SampleBrowserPlugPreset {
    pub gain_db: f32,
}

impl Default for SampleBrowserPlugPreset {
    fn default() -> Self {
        Self { gain_db: 0.0 }
    }
}

pub struct SampleBrowserPlugHandle {
    to_audio_thread_tx: Producer<ProcessMsg>,
    host_request: HostRequest,
}

impl SampleBrowserPlugHandle {
    pub fn play_sample(&mut self, pcm: Shared<PcmResource>) {
        self.send(ProcessMsg::PlayNewSample { pcm });
        self.host_request.request_process();
    }

    pub fn replay_sample(&mut self) {
        self.send(ProcessMsg::ReplaySample);
        self.host_request.request_process();
    }

    pub fn stop(&mut self) {
        self.send(ProcessMsg::Stop);
    }

    pub fn discard_sample(&mut self) {
        self.send(ProcessMsg::DiscardSample);
    }

    fn send(&mut self, msg: ProcessMsg) {
        if let Err(e) = self.to_audio_thread_tx.push(msg) {
            log::error!("Sample browser plugin failed to send message: {}", e);
        }
    }
}

enum ProcessMsg {
    PlayNewSample { pcm: Shared<PcmResource> },
    ReplaySample,
    Stop,
    DiscardSample,
}

//unsafe impl Send for ProcessMsg {}
//unsafe impl Sync for ProcessMsg {}

struct ParamsHandle {
    pub gain: ParamF32Handle,
}

impl ParamsHandle {
    fn load_preset(&self, preset: &SampleBrowserPlugPreset) {
        self.gain.set_value(preset.gain_db);
    }
}

struct Params {
    pub gain: ParamF32,
}

impl Params {
    fn new(
        preset: &SampleBrowserPlugPreset,
        sample_rate: SampleRate,
        max_frames: usize,
    ) -> (Self, ParamsHandle) {
        let (gain, gain_handle) = ParamF32::from_value(
            preset.gain_db,
            0.0,
            -90.0,
            6.0,
            DEFAULT_DB_GRADIENT,
            Unit::Decibels,
            DEFAULT_SMOOTH_SECS,
            sample_rate,
            max_frames,
        );

        (Params { gain }, ParamsHandle { gain: gain_handle })
    }
}

pub struct SampleBrowserPlugMainThread {
    params: ParamsHandle,
    host_request: HostRequest,
}

impl SampleBrowserPlugMainThread {
    fn new(host_request: HostRequest) -> Self {
        // These parameters will be re-initialized later with the correct sample_rate
        // and max_frames when the plugin is activated.
        let (_params, params_handle) =
            Params::new(&SampleBrowserPlugPreset::default(), Default::default(), 0);

        Self { params: params_handle, host_request }
    }

    fn save_state(&self) -> SampleBrowserPlugPreset {
        SampleBrowserPlugPreset { gain_db: self.params.gain.value() }
    }
}

impl PluginMainThread for SampleBrowserPlugMainThread {
    fn activate(
        &mut self,
        sample_rate: SampleRate,
        _min_frames: u32,
        max_frames: u32,
        coll_handle: &basedrop::Handle,
    ) -> Result<PluginActivatedInfo, String> {
        let preset = self.save_state();

        let (params, params_handle) = Params::new(&preset, sample_rate, max_frames as usize);
        self.params = params_handle;

        let (to_audio_thread_tx, from_handle_rx) = RingBuffer::<ProcessMsg>::new(MSG_BUFFER_SIZE);
        let from_handle_rx = Owned::new(coll_handle, from_handle_rx);

        Ok(PluginActivatedInfo {
            audio_thread: Box::new(SampleBrowserPlugAudioThread {
                params,
                from_handle_rx,
                pcm: None,
                is_playing: false,
                playhead: 0,
            }),
            internal_handle: Some(Box::new(SampleBrowserPlugHandle {
                to_audio_thread_tx,
                host_request: self.host_request.clone(),
            })),
        })
    }

    fn collect_save_state(&mut self) -> Result<Option<Vec<u8>>, String> {
        let preset: Vec<u8> =
            bincode::serialize(&self.save_state()).map_err(|e| format!("{}", e))?;

        Ok(Some(preset))
    }

    fn load_state(&mut self, preset: &PluginPreset) -> Result<(), String> {
        let decoded_preset = bincode::deserialize(&preset.bytes).map_err(|e| format!("{}", e))?;

        self.params.load_preset(&decoded_preset);

        Ok(())
    }

    fn audio_ports_ext(&mut self) -> Result<ext::audio_ports::PluginAudioPortsExt, String> {
        Ok(ext::audio_ports::PluginAudioPortsExt::stereo_out())
    }

    // --- Parameters ---------------------------------------------------------------------------------

    fn num_params(&mut self) -> u32 {
        1
    }

    fn param_info(&mut self, param_index: usize) -> Result<ext::params::ParamInfo, ()> {
        match param_index {
            0 => Ok(ext::params::ParamInfo::new(
                ParamID(0),
                ParamInfoFlags::default_float(),
                "gain".into(),
                String::new(),
                -90.0,
                6.0,
                0.0,
            )),
            _ => Err(()),
        }
    }

    fn param_value(&self, param_id: ParamID) -> Result<f64, ()> {
        match param_id {
            ParamID(0) => Ok(f64::from(self.params.gain.value())),
            _ => Err(()),
        }
    }

    fn param_value_to_text(&self, param_id: ParamID, value: f64) -> Result<String, ()> {
        match param_id {
            ParamID(0) => Ok(default_db_value_to_text(value)),
            _ => Err(()),
        }
    }

    fn param_text_to_value(&self, param_id: ParamID, text: &str) -> Result<f64, ()> {
        match param_id {
            ParamID(0) => parse_text_to_f64(text),
            _ => Err(()),
        }
    }
}

pub struct SampleBrowserPlugAudioThread {
    params: Params,

    from_handle_rx: Owned<Consumer<ProcessMsg>>,
    pcm: Option<Shared<PcmResource>>,

    is_playing: bool,
    playhead: usize,
}

impl SampleBrowserPlugAudioThread {
    fn poll(&mut self, in_events: &EventBuffer) {
        for e in in_events.iter() {
            if let Some(param_value) = e.as_event::<ParamValueEvent>() {
                if param_value.param_id() == 0 {
                    self.params.gain.set_value(param_value.value() as f32)
                }
            }
        }

        while let Ok(msg) = self.from_handle_rx.pop() {
            match msg {
                ProcessMsg::PlayNewSample { pcm } => {
                    self.pcm = Some(pcm);
                    self.is_playing = true;
                    self.playhead = 0;
                }
                ProcessMsg::ReplaySample => {
                    self.is_playing = true;
                    self.playhead = 0;
                }
                ProcessMsg::Stop => {
                    self.is_playing = false;
                }
                ProcessMsg::DiscardSample => {
                    self.is_playing = false;
                    self.pcm = None;
                }
            }
        }
    }
}

impl PluginAudioThread for SampleBrowserPlugAudioThread {
    fn start_processing(&mut self) -> Result<(), ()> {
        Ok(())
    }

    fn stop_processing(&mut self) {}

    fn process(
        &mut self,
        proc_info: &ProcInfo,
        buffers: &mut ProcBuffers,
        in_events: &EventBuffer,
        _out_events: &mut EventBuffer,
    ) -> ProcessStatus {
        self.poll(in_events);

        if self.is_playing && self.pcm.is_some() {
            let pcm = self.pcm.as_ref().unwrap();

            if self.playhead < pcm.len_frames.0 as usize {
                let (mut buf_l, mut buf_r) = buffers.audio_out[0].stereo_f32_mut().unwrap();

                let buf_l_part = &mut buf_l[0..proc_info.frames];
                let buf_r_part = &mut buf_r[0..proc_info.frames];

                pcm.fill_stereo_f32(self.playhead as isize, buf_l_part, buf_r_part);

                for i in 0..proc_info.frames {
                    buf_l_part[i] *= 0.5;
                    buf_r_part[i] *= 0.5;
                }

                self.playhead += proc_info.frames;

                return ProcessStatus::Continue;
            } else {
                self.is_playing = false;
            }
        }

        buffers.audio_out[0].clear_all(proc_info.frames);

        ProcessStatus::Sleep
    }

    fn param_flush(&mut self, in_events: &EventBuffer, _out_events: &mut EventBuffer) {
        self.poll(in_events);
    }
}
