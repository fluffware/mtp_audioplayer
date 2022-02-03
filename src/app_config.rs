use crate::actions::action::Action;
use crate::state_machine::StateMachine;
use crate::actions::{
    debug::DebugAction,
    parallel::ParallelAction,
    play::PlayAction,
    repeat::RepeatAction,
    sequence::SequenceAction,
    tag_dispatcher::{self, TagDispatched, TagDispatcher},
    alarm_dispatcher::{self, AlarmDispatched, AlarmDispatcher},
    wait::WaitAction,
    wait_tag::WaitTagAction,
    wait_alarm::WaitAlarmAction,
    goto::GotoAction,
};
use crate::alarm_filter::BoolOp as AlarmBoolOp;
use crate::clip_queue::ClipQueue;
use crate::open_pipe::alarm_data::AlarmData;
use crate::read_config::{ActionType};
use crate::open_pipe::alarm_data::AlarmId;
use crate::{
    clip_player::ClipPlayer,
    read_config::{ClipType, PlayerConfig},
};
use log::debug;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

type DynResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn load_clip(os_file: &Path) -> DynResult<Arc<Vec<i16>>> {
    let mut samples;
    match hound::WavReader::open(&os_file) {
        Ok(mut reader) => {
            samples = Vec::<i16>::new();
            for s in reader.samples::<i16>() {
                match s {
                    Ok(s) => samples.push(s),
                    Err(err) => {
                        return Err(format!(
                            "Failed to read samples from file \"{}\": {}",
                            os_file.to_string_lossy(),
                            err
                        )
                        .into())
                    }
                }
            }
        }
        Err(err) => {
            return Err(format!(
                "Failed to open audio file \"{}\": {}",
                os_file.to_string_lossy(),
                err
            )
            .into())
        }
    }
    Ok(Arc::new(samples))
}

const SAMPLE_MAX: f64 = std::i16::MAX as f64;

pub fn load_clips(
    clip_root: &Path,
    clip_conf: &HashMap<String, ClipType>,
    rate: u32,
    channels: u8,
) -> DynResult<HashMap<String, Arc<Vec<i16>>>> {
    let mut clips = HashMap::<String, Arc<Vec<i16>>>::new();
    for (name, conf) in clip_conf {
        match conf {
            ClipType::File(f) => {
                let os_name = clip_root.join(f);
                let samples = load_clip(&os_name)?;
                clips.insert(name.clone(), samples);
            }
            ClipType::Sine {
                amplitude,
                frequency,
                duration,
            } => {
                let rate = f64::from(rate);
                let ramp = 100;
                let scale = amplitude * SAMPLE_MAX;
                let length = (rate * duration.as_secs_f64()).round() as usize;
                let mut samples = Vec::<i16>::with_capacity(length * usize::from(channels));
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
                    let s = (f64::sin((i as f64) * fscale) * env) as i16;
                    for _ in 0..channels {
                        samples.push(s);
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
    pub clips: HashMap<String, Arc<Vec<i16>>>,
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
        player_conf.rate,
        player_conf.channels,
    )?;
    let rate = player_conf.rate;
    let channels = player_conf.channels;
    let clip_player = ClipPlayer::new(&player_conf.playback_device, rate, channels)
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

struct ActionBuildData<'a>
{
    playback_ctxt: &'a PlaybackContext,
    tag_ctxt: &'a Arc<TagContext>,
    alarm_ctxt: &'a Arc<AlarmContext>,
    state_machine_map: &'a HashMap<String, Arc<StateMachine>>,
    current_state_machine: &'a Arc<StateMachine>,
}

fn action_conf_to_action(
    build_data: &ActionBuildData,
    action_conf: &ActionType,    
) -> DynResult<Arc<dyn Action + Send + Sync>> {
    match action_conf {
        ActionType::Sequence(conf_actions) => {
            let mut sequence = SequenceAction::new();
            for conf_action in conf_actions {
                let action =
                    action_conf_to_action(build_data, conf_action)?;
                sequence.add_arc_action(action);
            }
            Ok(Arc::new(sequence))
        }
        ActionType::Parallel(conf_actions) => {
            let mut parallel = ParallelAction::new();
            for conf_action in conf_actions {
                let action =
                    action_conf_to_action(build_data, conf_action)?;
                parallel.add_arc_action(action);
            }
            Ok(Arc::new(parallel))
        }
        ActionType::Play {
            priority,
            timeout,
            sound,
        } => {
            let samples = build_data.playback_ctxt
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
            Ok(Arc::new(RepeatAction::new(repeated, *count)))
        }
        ActionType::Goto(state_name) => {
            let state_machine;
            let state_name_ref;
            if let Some((machine, name)) = state_name.split_once(":") {
                state_machine = match build_data.state_machine_map.get(name) {
                    Some(s) => s,
                    None => return Err(format!("No state machine named '{}'", machine).into())
                };
                state_name_ref = name;
            } else {
                state_machine = build_data.current_state_machine;
                state_name_ref = state_name;
            }
            let state_index = match state_machine.find_state_index(state_name_ref) {
                Some(s) => s,
                None => return Err(format!("No state named '{}' in state machine '{}'", state_name, state_machine.name).into())
            };
            Ok(Arc::new(GotoAction::new(state_index, Arc::downgrade(state_machine))))
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
        ActionType::Debug(text) => Ok(Arc::new(DebugAction::new(text.clone()))),
    }
}


struct TagObservable {
    state: Option<String>,
    observers: Vec<oneshot::Sender<String>>,
}

pub struct TagContext {
    tags: Mutex<HashMap<String, TagObservable>>,
}

impl TagContext {
    pub fn new() -> TagContext {
        TagContext {
            tags: Mutex::new(HashMap::new()),
        }
    }

    pub fn tag_changed(&self, name: &str, new_value: &str) {
        debug!("{}: -> {}", name, new_value);
        if let Ok(mut tags) = self.tags.lock() {
            if let Some(data) = tags.get_mut(name) {
                data.state = Some(new_value.to_string());
                for obs in data.observers.drain(0..) {
                    let _ = obs.send(new_value.to_string());
                }
            }
        }
    }

    pub fn tag_names(&self) -> Vec<String> {
        let tags = self.tags.lock().unwrap();
        tags.keys().cloned().collect()
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
            .ok_or_else(|| tag_dispatcher::Error::TagNotFound)?;
        let value = data.state.clone();
        let (tx, rx) = oneshot::channel();
        data.observers.push(tx);
        let wait_tag = Box::pin(async {
            rx.await
                .map_err(|_| tag_dispatcher::Error::DispatcherNotAvailable)
        });
        Ok((value, wait_tag))
    }

    fn get_value(&self, tag: &str) -> Option<String> {
        let mut tags = self.tags.lock().ok()?;
        let data = tags.get_mut(tag)?;
        data.state.clone()
    }
}

pub fn setup_tags(player_conf: &PlayerConfig) -> DynResult<TagContext> {
    let tag_ctxt = TagContext::new();
    {
        let mut tags = tag_ctxt.tags.lock().unwrap();
        for name in &player_conf.tags {
            tags.insert(
                name.to_string(),
                TagObservable {
                    state: None,
                    observers: Vec::new(),
                },
            );
        }
    }
    Ok(tag_ctxt)
}


struct AlarmFilterState {
    
    filter: Box<AlarmBoolOp>,
    matching: HashSet<AlarmId>,
    observers: Vec<oneshot::Sender<u32>>,    
}

impl AlarmFilterState {
    pub fn handle_notification(&mut self, new_alarm: &AlarmData) -> DynResult<()> {
        if self.filter.evaluate(new_alarm) {
            if self.matching.insert(AlarmId::from(new_alarm)) {
                let count = self.matching.len();
                for obs in self.observers.drain(0..) {
                    let _ = obs.send(count as u32);
                }
            }
        } else {
            if self.matching.remove(&AlarmId::from(new_alarm)) {
                let count = self.matching.len();
                for obs in self.observers.drain(0..) {
                    let _ = obs.send(count as u32);
                }
            }
        }
        Ok(())
    }
}

pub struct AlarmContext {
    alarm_filters: Mutex<HashMap<String, AlarmFilterState>>
}

impl AlarmContext {
    pub fn handle_notification(&self, new_alarm: &AlarmData) -> DynResult<()> {
        let mut filters = self.alarm_filters.lock().map_err(|e| format!("Failed to lock alarm filters: {}", e))?;
        for filter in filters.values_mut() {
            filter.handle_notification(new_alarm)?;
        }
        Ok(())
    }
}

impl AlarmDispatcher for AlarmContext
{
    
    fn wait_alarm_filter(&self, filter_name: &str) -> Result<(u32, AlarmDispatched), alarm_dispatcher::Error>
    {
        let mut filters = self
        .alarm_filters
            .lock()
            .map_err(|_| alarm_dispatcher::Error::DispatcherNotAvailable)?;
        let filter = filters
            .get_mut(filter_name)
            .ok_or_else(|| alarm_dispatcher::Error::AlarmFilterNotFound)?;
        let count = filter.matching.len();
        let (tx, rx) = oneshot::channel();
        filter.observers.push(tx);
        let wait_alarm = Box::pin(async {
            rx.await
                .map_err(|_| alarm_dispatcher::Error::DispatcherNotAvailable)
        });
        Ok((count as u32, wait_alarm))
    }
    
    
    
    fn get_filter_count(&self, filter_name: &str) -> Result<u32, alarm_dispatcher::Error>
    {
        let mut filters = self
            .alarm_filters
            .lock()
            .map_err(|_| alarm_dispatcher::Error::DispatcherNotAvailable)?;
        let filter = filters
            .get_mut(filter_name)
            .ok_or_else(|| alarm_dispatcher::Error::AlarmFilterNotFound)?;
        Ok(filter.matching.len() as u32)
    }
}

pub fn setup_alarms(
    player_conf: &PlayerConfig,
) -> DynResult<AlarmContext> {
    let mut alarm_filters = HashMap::new();

    for (name, op) in &player_conf.named_alarm_filters {
        let filter_state =AlarmFilterState{filter: Box::new(op.clone()), matching: HashSet::new(), observers: Vec::new()};
        alarm_filters.insert(name.to_string(), filter_state);
    }
    let alarm_ctxt = AlarmContext {
        alarm_filters: Mutex::new(alarm_filters),
    };
    Ok(alarm_ctxt)
}

pub struct StateMachineContext {
    state_machines: Vec<Arc<StateMachine>>
}

impl StateMachineContext
{
    pub async fn start_all(&self)
    {
        for sm in &self.state_machines {
            sm.goto(0).await;
        }
    }
}
pub fn setup_state_machines(
    player_conf: &PlayerConfig,
    playback_ctxt: &PlaybackContext,
    tag_ctxt: &Arc<TagContext>,
    alarm_ctxt: &Arc<AlarmContext>,
) -> DynResult<StateMachineContext> {
    let mut state_machines = Vec::new();
    let mut state_machine_map = HashMap::new();
    for state_machine_conf in &player_conf.state_machines {
        let state_machine = StateMachine::new(&state_machine_conf.id);
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
            //let named_actions = &action_ctxt.actions;
            let build_data = ActionBuildData {
                playback_ctxt, 
                tag_ctxt, alarm_ctxt,state_machine_map: &state_machine_map, current_state_machine: state_machine
            };
            let action =  action_conf_to_action(&build_data, action_conf)?;
            state_machine.set_action(state_index, action);
            state_machines.push(state_machine.clone());
                
        }
    }
    Ok(StateMachineContext {state_machines})
}
