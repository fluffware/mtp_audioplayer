use crate::actions::action::Action;
use crate::actions::{
    alarm_dispatcher::{self, AlarmDispatched, AlarmDispatcher},
    alarm_function::AlarmFunctionAction,
    alarm_function::AlarmOp,
    alarm_functions::AlarmFunctions,
    debug::DebugAction,
    goto::GotoAction,
    parallel::ParallelAction,
    play::PlayAction,
    repeat::RepeatAction,
    sequence::SequenceAction,
    set_tag::SetTagAction,
    set_volume::SetVolumeAction,
    tag_dispatcher::{self, TagDispatched, TagDispatcher},
    tag_setter::{TagSetFuture, TagSetter},
    wait::WaitAction,
    wait_alarm::WaitAlarmAction,
    wait_tag::WaitTagAction,
};
use crate::alarm_filter::BoolOp as AlarmBoolOp;
use crate::clip_queue::ClipQueue;
use crate::event_limit::EventLimit;
use crate::open_pipe::alarm_data::AlarmData;
use crate::open_pipe::alarm_data::AlarmId;
use crate::read_config::ActionType;
use crate::read_config::TagOrConst;
use crate::sample_buffer::{Sample as BufferSample, SampleBuffer};
use crate::state_machine::StateMachine;
use crate::util::error::DynResult;
use crate::volume_control::VolumeControl;
use crate::{
    clip_player::ClipPlayer,
    read_config::{ClipType, PlayerConfig},
};

use cpal::SampleFormat;
use log::{debug, error};
use simple_samplerate::{sample::Sample, samplerate::Samplerate};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::time::{timeout, Duration};

const BLOCK_SIZE: usize = 1024;

fn convert_samples<S, R>(
    reader: &mut hound::WavReader<R>,
    file_name: &Path,
    from_rate: u32,
    to_rate: u32,
    channels: usize,
    amplitude: f32,
) -> DynResult<Vec<S>>
where
    R: Read,
    S: Clone + BufferSample + Sample,
{
    let mut conv = Samplerate::new(from_rate, to_rate, channels).unwrap();
    let mut input = Vec::<f32>::new();
    let mut out_buffer: Vec<S> = Vec::new();
    let out_block_size = BLOCK_SIZE * to_rate as usize / from_rate as usize + 8 * channels;
    let scale = amplitude / 32767.0;
    for s in reader.samples::<i16>() {
        match s {
            Ok(s) => {
                let s = f32::from(s) * scale;
                input.push(s);
                if input.len() >= BLOCK_SIZE {
                    //println!("Writing block");
                    let start = out_buffer.len();
                    out_buffer.resize(out_block_size + start, S::SAMPLE_OFFSET);
                    let count = conv.process_buffer(&input, &mut out_buffer[start..]);
                    out_buffer.truncate(count + start);
                    input.clear();
                }
            }
            Err(err) => {
                return Err(format!(
                    "Failed to read samples from file \"{}\": {}",
                    file_name.to_string_lossy(),
                    err
                )
                .into())
            }
        }
    }
    Ok(out_buffer)
}

fn load_clip(
    os_file: &Path,
    sample_format: SampleFormat,
    sample_rate: u32,
    channels: usize,
    amplitude: f32,
) -> DynResult<Arc<SampleBuffer>> {
    let mut reader = hound::WavReader::open(os_file)
        .map_err::<Box<dyn std::error::Error + Send + Sync>, _>(|err| {
            format!(
                "Failed to open audio file \"{}\": {}",
                os_file.to_string_lossy(),
                err
            )
            .into()
        })?;
    let spec = reader.spec();

    let samples = match sample_format {
        SampleFormat::I16 => SampleBuffer::I16(convert_samples(
            &mut reader,
            os_file,
            spec.sample_rate,
            sample_rate,
            channels,
            amplitude,
        )?),
        SampleFormat::U16 => SampleBuffer::U16(convert_samples(
            &mut reader,
            os_file,
            spec.sample_rate,
            sample_rate,
            channels,
            amplitude,
        )?),
        SampleFormat::F32 => SampleBuffer::F32(convert_samples(
            &mut reader,
            os_file,
            spec.sample_rate,
            sample_rate,
            channels,
            amplitude,
        )?),
    };

    Ok(Arc::new(samples))
}

pub fn load_clips(
    clip_root: &Path,
    clip_conf: &HashMap<String, ClipType>,
    sample_format: SampleFormat,
    rate: u32,
    channels: u8,
) -> DynResult<HashMap<String, Arc<SampleBuffer>>> {
    let mut clips = HashMap::<String, Arc<SampleBuffer>>::new();
    for (name, conf) in clip_conf {
        match conf {
            ClipType::File {
                file_name,
                amplitude,
            } => {
                let os_name = clip_root.join(file_name);
                let samples =
                    load_clip(&os_name, sample_format, rate, channels as usize, *amplitude)?;
                clips.insert(name.clone(), samples);
            }
            ClipType::Sine {
                amplitude,
                frequency,
                duration,
            } => {
                let rate = f64::from(rate);
                let ramp = 100;
                let length = (rate * duration.as_secs_f64()).round() as usize;
                let sample_max;
                let sample_offset;
                let mut samples;
                match sample_format {
                    SampleFormat::I16 => {
                        sample_max = i16::SAMPLE_MAX as f64;
                        sample_offset = i16::SAMPLE_OFFSET as f64;
                        samples = SampleBuffer::I16(Vec::<i16>::with_capacity(
                            length * usize::from(channels),
                        ));
                    }
                    SampleFormat::U16 => {
                        sample_max = u16::SAMPLE_MAX as f64;
                        sample_offset = u16::SAMPLE_OFFSET as f64;
                        samples = SampleBuffer::U16(Vec::<u16>::with_capacity(
                            length * usize::from(channels),
                        ));
                    }
                    SampleFormat::F32 => {
                        sample_max = f32::SAMPLE_MAX as f64;
                        sample_offset = f32::SAMPLE_OFFSET as f64;
                        samples = SampleBuffer::F32(Vec::<f32>::with_capacity(
                            length * usize::from(channels),
                        ))
                    }
                }
                let scale = amplitude * sample_max;

                let fscale = frequency * std::f64::consts::TAU / rate;
                for i in 0..length {
                    let env;
                    if i < ramp {
                        env = scale * (i as f64) / (ramp as f64);
                    } else if i > length - ramp {
                        env = scale * ((length - i) as f64) / (ramp as f64);
                    } else {
                        env = scale;
                    }
                    let s = f64::sin((i as f64) * fscale) * env + sample_offset;
                    for _ in 0..channels {
                        match &mut samples {
                            SampleBuffer::I16(buf) => buf.push(s as i16),
                            SampleBuffer::U16(buf) => buf.push(s as u16),
                            SampleBuffer::F32(buf) => buf.push(s as f32),
                        }
                    }
                }

                clips.insert(name.clone(), Arc::new(samples));
            }
        }
    }
    Ok(clips)
}

#[derive(Debug)]
pub enum PlaybackError {
    NameNotFound(String),
}

impl std::error::Error for PlaybackError {}

impl std::fmt::Display for PlaybackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        match self {
            Self::NameNotFound(name) => write!(f, "Clip '{}' not found", name),
        }
    }
}

pub struct PlaybackContext {
    pub rate: u32,
    pub channels: u8,
    pub clip_queue: Arc<ClipQueue>,
    pub clips: HashMap<String, Arc<SampleBuffer>>,
}

impl PlaybackContext {
    pub async fn play(&self, clip_name: &str, priority: i32) -> DynResult<()> {
        let clip = self
            .clips
            .get(clip_name)
            .ok_or_else(|| PlaybackError::NameNotFound(clip_name.to_string()))?;
        self.clip_queue.play(clip.clone(), priority, None).await?;

        Ok(())
    }
}

pub fn setup_clip_playback(
    player_conf: &PlayerConfig,
    base_dir: &Path,
) -> DynResult<PlaybackContext> {
    let clip_root = base_dir.join(&player_conf.clip_root);
    let clips = load_clips(
        &clip_root,
        &player_conf.clips,
        player_conf.sample_format,
        player_conf.rate,
        player_conf.channels,
    )?;
    let rate = player_conf.rate;
    let channels = player_conf.channels;
    let sample_format = player_conf.sample_format;
    let clip_player = ClipPlayer::new(&player_conf.playback_device, rate, channels, sample_format)
        .map_err(|e| format!("Failed to initialise playback: {}", e))?;

    let clip_queue = ClipQueue::new(clip_player);
    Ok(PlaybackContext {
        rate,
        channels,
        clip_queue: Arc::new(clip_queue),
        clips,
    })
}

pub struct ActionContext {
    pub actions: HashMap<String, Arc<dyn Action + Send + Sync>>,
}

struct ActionBuildData<'a> {
    playback_ctxt: &'a PlaybackContext,
    tag_ctxt: &'a Arc<TagContext>,
    volume_control: &'a Arc<VolumeControlContext>,
    alarm_ctxt: &'a Arc<AlarmContext>,
    state_machine_map: &'a HashMap<String, Arc<StateMachine>>,
    current_state_machine: &'a Arc<StateMachine>,
    repeat_limit: EventLimit,
}

fn action_conf_to_action(
    build_data: &ActionBuildData,
    action_conf: &ActionType,
) -> DynResult<Arc<dyn Action + Send + Sync>> {
    match action_conf {
        ActionType::Sequence(conf_actions) => {
            let mut sequence = SequenceAction::new();
            for conf_action in conf_actions {
                let action = action_conf_to_action(build_data, conf_action)?;
                sequence.add_arc_action(action);
            }
            Ok(Arc::new(sequence))
        }
        ActionType::Parallel(conf_actions) => {
            let mut parallel = ParallelAction::new();
            for conf_action in conf_actions {
                let action = action_conf_to_action(build_data, conf_action)?;
                parallel.add_arc_action(action);
            }
            Ok(Arc::new(parallel))
        }
        ActionType::Play {
            priority,
            timeout,
            sound,
        } => {
            let samples = build_data
                .playback_ctxt
                .clips
                .get(sound)
                .ok_or_else(|| format!("No clip named '{}'", sound))?;
            let action = PlayAction::new(
                build_data.playback_ctxt.clip_queue.clone(),
                *priority,
                *timeout,
                samples.clone(),
            );
            Ok(Arc::new(action))
        }
        ActionType::Wait(timeout) => Ok(Arc::new(WaitAction::new(*timeout))),
        ActionType::Repeat { count, action } => {
            let repeated = action_conf_to_action(build_data, action)?;
            Ok(Arc::new(RepeatAction::new(repeated, *count, build_data.repeat_limit.clone())))
        }
        ActionType::Goto(state_name) => {
            let state_machine;
            let state_name_ref;
            if let Some((machine, name)) = state_name.split_once(':') {
                state_machine = match build_data.state_machine_map.get(machine) {
                    Some(s) => s,
                    None => return Err(format!("No state machine named '{}'", machine).into()),
                };
                state_name_ref = name;
            } else {
                state_machine = build_data.current_state_machine;
                state_name_ref = state_name;
            }
            let state_index = match state_machine.find_state_index(state_name_ref) {
                Some(s) => s,
                None => {
                    return Err(format!(
                        "No state named '{}' in state machine '{}'",
                        state_name, state_machine.name
                    )
                    .into())
                }
            };
            Ok(Arc::new(GotoAction::new(
                state_index,
                Arc::downgrade(state_machine),
            )))
        }
        ActionType::WaitTag {
            tag_name,
            condition,
        } => Ok(Arc::new(WaitTagAction::new(
            tag_name.clone(),
            condition.clone(),
            build_data.tag_ctxt.clone(),
        ))),
        ActionType::WaitAlarm {
            filter_name,
            condition,
        } => Ok(Arc::new(WaitAlarmAction::new(
            filter_name.clone(),
            condition.clone(),
            build_data.alarm_ctxt.clone(),
        ))),
        ActionType::SetTag { tag_name, value } => Ok(Arc::new(SetTagAction::new(
            tag_name.clone(),
            value.clone(),
            build_data.tag_ctxt.clone(),
        ))),
        ActionType::IgnoreAlarms {
            filter,
            permanent: _,
        } => Ok(Arc::new(AlarmFunctionAction::new(
            filter.clone(),
            build_data.alarm_ctxt.clone(),
            AlarmOp::Ignore,
        ))),
        ActionType::RestoreAlarms { filter } => Ok(Arc::new(AlarmFunctionAction::new(
            filter.clone(),
            build_data.alarm_ctxt.clone(),
            AlarmOp::Restore,
        ))),

        ActionType::Debug(text) => Ok(Arc::new(DebugAction::new(text.clone()))),
        ActionType::SetVolume { control, value } => {
            let ctrl = match build_data.volume_control.controls.get(control) {
                Some(ctrl) => ctrl,
                None => return Err(format!("No volume control named '{}' found.", control).into()),
            };
            match value {
                TagOrConst::Tag(tag_name) => Ok(Arc::new(SetVolumeAction::new_tag(
                    ctrl.clone(),
                    tag_name.to_string(),
                    build_data.tag_ctxt.clone(),
                ))),
                TagOrConst::Const(level) => Ok(Arc::new(SetVolumeAction::<TagContext>::new_const(
                    ctrl.clone(),
                    *level,
                ))),
            }
        }
    }
}

pub struct TagSetRequest {
    pub tag_name: String,
    pub value: String,
    pub done: oneshot::Sender<DynResult<()>>,
}

struct TagObservable {
    state: Option<String>,
    observers: (watch::Sender<String>, watch::Receiver<String>),
}

pub struct TagContext {
    tags: Mutex<HashMap<String, TagObservable>>,
    tag_send_tx: UnboundedSender<TagSetRequest>,
}

impl TagContext {
    pub fn new(tag_send_tx: UnboundedSender<TagSetRequest>) -> TagContext {
        TagContext {
            tags: Mutex::new(HashMap::new()),
            tag_send_tx,
        }
    }

    pub fn tag_changed(&self, name: &str, new_value: &str) {
        debug!("{}: -> {}", name, new_value);
        if let Ok(mut tags) = self.tags.lock() {
            if let Some(data) = tags.get_mut(name) {
                data.state = Some(new_value.to_string());
                if let Err(err) = data.observers.0.send(new_value.to_string()) {
                    error!("Failed to notify tag observers: {}", err);
                }
            }
        }
    }

    pub fn tag_names(&self) -> Vec<String> {
        let tags = self.tags.lock().unwrap();
        tags.keys().cloned().collect()
    }

    pub fn add_tag(&self, name: &str, state: Option<String>) {
        let mut tags = self.tags.lock().unwrap();
        tags.insert(
            name.to_string(),
            TagObservable {
                state,
                observers: watch::channel("".to_string()),
            },
        );
    }
}

impl TagSetter for TagContext {
    fn async_set_tag(&self, tag_name: &str, value: &str) -> TagSetFuture {
        self.tag_changed(tag_name, value);
        let (done_send, done_recv) = oneshot::channel();
        let req = TagSetRequest {
            tag_name: tag_name.to_string(),
            value: value.to_string(),
            done: done_send,
        };
        if self.tag_send_tx.send(req).is_err() {
            return Box::pin(std::future::ready(Err("Failed to queue request".into())));
        }
        Box::pin(async move { timeout(Duration::from_millis(500), done_recv).await?? })
    }

    fn set_tag(&self, tag_name: &str, value: &str) -> DynResult<()> {
        self.tag_changed(tag_name, value);
        let (done_send, _done_recv) = oneshot::channel();
        let req = TagSetRequest {
            tag_name: tag_name.to_string(),
            value: value.to_string(),
            done: done_send,
        };
        if self.tag_send_tx.send(req).is_err() {
            return Err("Failed to queue request".into());
        }
        Ok(())
    }
}

impl TagDispatcher for TagContext {
    fn wait_value(
        &self,
        tag: &str,
    ) -> Result<(Option<String>, TagDispatched), tag_dispatcher::Error> {
        let mut tags = self
            .tags
            .lock()
            .map_err(|_| tag_dispatcher::Error::DispatcherNotAvailable)?;
        let data = tags
            .get_mut(tag)
            .ok_or(tag_dispatcher::Error::TagNotFound)?;
        let value = data.state.clone();
        let mut rx = data.observers.1.clone();
        let wait_tag = Box::pin(async move {
            rx.borrow_and_update(); // Make sure that changed will block until next change
            rx.changed()
                .await
                .map_err(|_| tag_dispatcher::Error::DispatcherNotAvailable)?;
            Ok(rx.borrow().to_string())
        });
        Ok((value, wait_tag))
    }

    fn get_value(&self, tag: &str) -> Option<String> {
        let mut tags = self.tags.lock().ok()?;
        let data = tags.get_mut(tag)?;
        data.state.clone()
    }
}

pub fn setup_tags(
    player_conf: &PlayerConfig,
    tag_send_tx: UnboundedSender<TagSetRequest>,
) -> DynResult<TagContext> {
    let tag_ctxt = TagContext::new(tag_send_tx);
    {
        for name in &player_conf.tags {
            tag_ctxt.add_tag(name, None);
        }
    }
    Ok(tag_ctxt)
}

struct AlarmFilterState {
    filter: Box<AlarmBoolOp>,
    matching: HashSet<AlarmId>,
    ignore: HashSet<AlarmId>,
    ignore_permanent: bool,
    tag_setter: Weak<TagContext>,
    tag_matching: Option<String>,
    tag_ignored: Option<String>,
    observers: (watch::Sender<u32>, watch::Receiver<u32>),
}

impl AlarmFilterState {
    pub fn handle_notification(&mut self, new_alarm: &AlarmData) -> DynResult<()> {
        if new_alarm.state == 128 {
            return Ok(());
        }
        if self.filter.evaluate(new_alarm) {
            if self.matching.insert(AlarmId::from(new_alarm)) {
                self.update_alarm_counts();
            }
        } else {
            if !self.ignore_permanent {
                self.ignore.remove(&AlarmId::from(new_alarm));
            }
            if self.matching.remove(&AlarmId::from(new_alarm)) {
                self.update_alarm_counts();
            }
        }
        Ok(())
    }

    fn matching_count(&self) -> usize {
        self.matching.difference(&self.ignore).count()
    }

    fn update_tags(&self, matching: usize, ignored: usize) {
        if let Some(tag_setter) = Weak::upgrade(&self.tag_setter) {
            if let Some(tag_matching) = &self.tag_matching {
                let _ = tag_setter
                    .as_ref()
                    .set_tag(tag_matching, &matching.to_string());
            }
            if let Some(tag_ignored) = &self.tag_ignored {
                let _ = tag_setter.set_tag(tag_ignored, &ignored.to_string());
            }
        }
    }

    fn update_alarm_counts(&self) {
        let count = self.matching_count();
        if let Err(err) = self.observers.0.send(count as u32) {
            error!("Failed to notify alarm observers: {}", err);
        }
        self.update_tags(count, self.ignore.len());
    }
}

pub struct AlarmContext {
    alarm_filters: Mutex<HashMap<String, AlarmFilterState>>,
}

impl AlarmContext {
    pub fn handle_notification(&self, new_alarm: &AlarmData) -> DynResult<()> {
        let mut filters = self
            .alarm_filters
            .lock()
            .map_err(|e| format!("Failed to lock alarm filters: {}", e))?;
        for filter in filters.values_mut() {
            filter.handle_notification(new_alarm)?;
        }
        Ok(())
    }
}

impl AlarmDispatcher for AlarmContext {
    fn wait_alarm_filter(
        &self,
        filter_name: &str,
    ) -> Result<(u32, AlarmDispatched), alarm_dispatcher::Error> {
        let mut filters = self
            .alarm_filters
            .lock()
            .map_err(|_| alarm_dispatcher::Error::DispatcherNotAvailable)?;
        let filter = filters
            .get_mut(filter_name)
            .ok_or(alarm_dispatcher::Error::AlarmFilterNotFound)?;
        let count = filter.matching_count();
        let mut rx = filter.observers.1.clone();
        let wait_alarm = Box::pin(async move {
            rx.borrow_and_update(); // Make sure that changed will block until next change
            rx.changed()
                .await
                .map_err(|_| alarm_dispatcher::Error::DispatcherNotAvailable)?;
            Ok(*rx.borrow())
        });
        Ok((count as u32, wait_alarm))
    }

    fn get_filter_count(&self, filter_name: &str) -> Result<u32, alarm_dispatcher::Error> {
        let mut filters = self
            .alarm_filters
            .lock()
            .map_err(|_| alarm_dispatcher::Error::DispatcherNotAvailable)?;
        let filter = filters
            .get_mut(filter_name)
            .ok_or(alarm_dispatcher::Error::AlarmFilterNotFound)?;
        Ok(filter.matching_count() as u32)
    }
}

impl AlarmFunctions for AlarmContext {
    fn ignore_matched_alarms(&self, filter: &str, permanent: bool) {
        if let Ok(mut filters) = self.alarm_filters.lock() {
            if let Some(filter) = filters.get_mut(filter) {
                filter.ignore = filter.matching.clone();
                filter.ignore_permanent = permanent;
                filter.update_alarm_counts();
            }
        }
    }

    fn restore_ignored_alarms(&self, filter: &str) {
        if let Ok(mut filters) = self.alarm_filters.lock() {
            if let Some(filter) = filters.get_mut(filter) {
                filter.ignore = HashSet::new();
                filter.update_alarm_counts();
            }
        }
    }
}

pub fn setup_alarms(
    player_conf: &PlayerConfig,
    tag_setter: Weak<TagContext>,
) -> DynResult<AlarmContext> {
    let mut alarm_filters = HashMap::new();

    for (name, filter_conf) in &player_conf.named_alarm_filters {
        let tag_setter = if filter_conf.tag_matching.is_none() && filter_conf.tag_ignored.is_none()
        {
            Weak::new()
        } else {
            tag_setter.clone()
        };
        let filter_state = AlarmFilterState {
            filter: Box::new(filter_conf.filter_predicate.clone()),
            matching: HashSet::new(),
            observers: watch::channel(0),
            ignore: HashSet::new(),
            ignore_permanent: false,
            tag_setter,
            tag_matching: filter_conf.tag_matching.clone(),
            tag_ignored: filter_conf.tag_ignored.clone(),
        };
        alarm_filters.insert(name.to_string(), filter_state);
    }
    let alarm_ctxt = AlarmContext {
        alarm_filters: Mutex::new(alarm_filters),
    };
    Ok(alarm_ctxt)
}

pub struct StateMachineContext {
    state_machines: Vec<Arc<StateMachine>>,
}

impl StateMachineContext {
    pub async fn run_all(&self) -> DynResult<()> {
        let mut running = Vec::new();
        for sm in &self.state_machines {
            running.push(sm.run());
        }
        let _ = futures::future::try_join_all(running).await?;
        Ok(())
    }
}
pub struct VolumeControlContext {
    controls: HashMap<String, Arc<Mutex<VolumeControl>>>,
}

pub fn setup_volume_control(player_conf: &PlayerConfig) -> DynResult<VolumeControlContext> {
    let mut ctxt = VolumeControlContext {
        controls: HashMap::new(),
    };

    for conf in &player_conf.volume_config {
        let control = VolumeControl::new(&conf.device)?;
        if let Some(volume) = conf.initial_volume {
            control.set_volume(volume)?;
        }
        ctxt.controls
            .insert(conf.id.clone(), Arc::new(Mutex::new(control)));
    }
    Ok(ctxt)
}

pub fn setup_state_machines(
    player_conf: &PlayerConfig,
    playback_ctxt: &PlaybackContext,
    tag_ctxt: &Arc<TagContext>,
    volume_control: &Arc<VolumeControlContext>,
    alarm_ctxt: &Arc<AlarmContext>,
    change_limit: EventLimit,
) -> DynResult<StateMachineContext> {
    let mut state_machines = Vec::new();
    let mut state_machine_map = HashMap::new();
    for state_machine_conf in &player_conf.state_machines {
        let state_machine = StateMachine::new(&state_machine_conf.id, change_limit.clone());
        for state_conf in &state_machine_conf.states {
            state_machine.add_state(&state_conf.id);
            debug!("Added: {}:{}", state_machine_conf.id, state_conf.id);
        }
        state_machine_map.insert(state_machine_conf.id.to_string(), state_machine);
    }

    for state_machine_conf in &player_conf.state_machines {
        let state_machine = state_machine_map.get(&state_machine_conf.id).unwrap();
        for state_conf in &state_machine_conf.states {
            let state_index = state_machine.find_state_index(&state_conf.id).unwrap();
            let action_conf = &state_conf.action;
            let build_data = ActionBuildData {
                playback_ctxt,
                tag_ctxt,
                volume_control,
                alarm_ctxt,
                state_machine_map: &state_machine_map,
                current_state_machine: state_machine,
		repeat_limit: change_limit.clone(),
            };
            let action = action_conf_to_action(&build_data, action_conf)?;
            state_machine.set_action(state_index, action);
        }
        state_machines.push(state_machine.clone());
    }
    Ok(StateMachineContext { state_machines })
}
