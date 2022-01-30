use crate::actions::action::Action;
use crate::actions::{
    debug::DebugAction,
    parallel::ParallelAction,
    play::PlayAction,
    repeat::RepeatAction,
    sequence::SequenceAction,
    tag_dispatcher::{self, TagDispatched, TagDispatcher},
    wait::WaitAction,
    wait_tag::WaitTagAction,
};
use crate::alarm_filter::BoolOp as AlarmBoolOp;
use crate::clip_queue::ClipQueue;
use crate::open_pipe::alarm_data::AlarmData;
use crate::read_config::{ActionType, AlarmTriggerType};
use crate::{
    clip_player::ClipPlayer,
    read_config::{ClipType, PlayerConfig},
};
use log::debug;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

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

fn action_conf_to_action(
    playback_ctxt: &PlaybackContext,
    tag_ctxt: &Arc<TagContext>,
    action_conf: &ActionType,
    action_map: &HashMap<String, Arc<dyn Action + Send + Sync>>,
) -> DynResult<Arc<dyn Action + Send + Sync>> {
    match action_conf {
        ActionType::Sequence(conf_actions) => {
            let mut sequence = SequenceAction::new();
            for conf_action in conf_actions {
                let action =
                    action_conf_to_action(playback_ctxt, tag_ctxt, conf_action, action_map)?;
                sequence.add_arc_action(action);
            }
            Ok(Arc::new(sequence))
        }
        ActionType::Parallel(conf_actions) => {
            let mut parallel = ParallelAction::new();
            for conf_action in conf_actions {
                let action =
                    action_conf_to_action(playback_ctxt, tag_ctxt, conf_action, action_map)?;
                parallel.add_arc_action(action);
            }
            Ok(Arc::new(parallel))
        }
        ActionType::Play {
            priority,
            timeout,
            sound,
        } => {
            let samples = playback_ctxt
                .clips
                .get(sound)
                .ok_or_else(|| format!("No clip named '{}'", sound))?;
            let action = PlayAction::new(
                playback_ctxt.clip_queue.clone(),
                *priority,
                *timeout,
                samples.clone(),
            );
            Ok(Arc::new(action))
        }
        ActionType::Wait(timeout) => Ok(Arc::new(WaitAction::new(*timeout))),
        ActionType::Reference(action_ref) => {
            let action = action_map
                .get(action_ref)
                .ok_or_else(|| format!("No preceding action with id {} found", action_ref))?;

            Ok(action.clone())
        }
        ActionType::Repeat { count, action } => {
            let repeated = action_conf_to_action(playback_ctxt, tag_ctxt, action, action_map)?;
            Ok(Arc::new(RepeatAction::new(repeated, *count)))
        }
        ActionType::AlarmRestart => Err("Alarm restart action not implemented".into()),
        ActionType::WaitTag {
            tag_name,
            condition,
        } => Ok(Arc::new(WaitTagAction::new(
            tag_name.clone(),
            condition.clone(),
            tag_ctxt.clone(),
        ))),
        ActionType::Debug(text) => Ok(Arc::new(DebugAction::new(text.clone()))),
    }
}

pub fn setup_actions(
    player_conf: &PlayerConfig,
    playback_ctxt: &PlaybackContext,
    tag_ctxt: &Arc<TagContext>,
) -> DynResult<ActionContext> {
    let mut actions = HashMap::new();
    for (name, action_conf) in &player_conf.named_actions {
        let action = action_conf_to_action(playback_ctxt, tag_ctxt, action_conf, &actions)?;
        actions.insert(name.clone(), action);
    }
    Ok(ActionContext { actions })
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
                .map_err(|e| tag_dispatcher::Error::DispatcherNotAvailable)
        });
        Ok((value, wait_tag))
    }

    fn get_value(&self, tag: &str) -> Option<String> {
        let mut tags = self.tags.lock().ok()?;
        let data = tags.get_mut(tag)?;
        data.state.clone()
    }
}

fn bool_value(s: &str) -> bool {
    let lcase = s.to_lowercase();
    if lcase == "false" {
        return false;
    } else if lcase == "true" {
        return true;
    }
    if s.parse().unwrap_or(0) != 0 {
        return true;
    }
    false
}

pub fn setup_tags(player_conf: &PlayerConfig) -> DynResult<TagContext> {
    let mut tag_ctxt = TagContext::new();
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

fn find_alarm_index(alarms: &[Rc<RefCell<AlarmData>>], key: &AlarmData) -> Result<usize, usize> {
    alarms.binary_search_by(|a| a.borrow().cmp_id(&key))
}

pub struct AlarmTrigger {
    trigger_type: AlarmTriggerType,
    filter: Box<AlarmBoolOp>,
    action: Arc<dyn Action + Send + Sync>,
    cancel: Option<CancellationToken>,
    matching: Vec<Rc<RefCell<AlarmData>>>,
}

impl AlarmTrigger {
    pub fn start(&mut self) {
        if self.cancel.is_some() {
            return;
        }
        let action = self.action.clone();
        let cancel = CancellationToken::new();
        self.cancel = Some(cancel.clone());
        tokio::spawn(async move {
            tokio::select! {
                _ = action.run() => {},
                _ = cancel.cancelled() => {},
            }
        });
        debug!("Trigger {} started", self.filter.to_string());
    }

    pub fn restart(&mut self) {
        self.stop();
        self.start();
    }

    pub fn stop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            debug!("Trigger {} stopped", self.filter.to_string());
            cancel.cancel();
        }
    }
}

impl Drop for AlarmTrigger {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
    }
}

pub struct AlarmContext {
    alarm_state: Vec<Rc<RefCell<AlarmData>>>,
    alarm_triggers: Vec<AlarmTrigger>,
}

impl AlarmContext {
    pub fn handle_notification(&mut self, new_alarm: AlarmData) -> DynResult<()> {
        let old_state;
        let new_state = new_alarm.state;
        let alarm_cell;
        match find_alarm_index(&self.alarm_state, &new_alarm) {
            Ok(p) => {
                old_state = self.alarm_state[p].borrow().state;
                self.alarm_state[p].borrow_mut().state = new_alarm.state;
                alarm_cell = &self.alarm_state[p];
            }
            Err(p) => {
                old_state = 0;
                self.alarm_state.insert(p, Rc::new(RefCell::new(new_alarm)));
                alarm_cell = &self.alarm_state[p];
            }
        }
        debug!("{} -> {}", old_state, new_state);
        if old_state != new_state {
            for trigger in &mut self.alarm_triggers {
                let res = find_alarm_index(&trigger.matching, &alarm_cell.borrow());
                if trigger.filter.evaluate(&alarm_cell.borrow()) {
                    debug!("Filter {} evaluated to true", trigger.filter.to_string());
                    if let Err(index) = res {
                        trigger.matching.insert(index, alarm_cell.clone());
                        match trigger.trigger_type {
                            AlarmTriggerType::WhileAnyActive => {
                                if !trigger.matching.is_empty() {
                                    debug!("Start any");
                                    trigger.start();
                                }
                            }
                            AlarmTriggerType::WhileNoneActive => {
                                debug!("Stop any");
                                trigger.stop();
                            }
                            AlarmTriggerType::WhenRaised => {
                                trigger.restart();
                            }
                            AlarmTriggerType::WhenFirstRaised => {
                                if trigger.matching.len() == 1 {
                                    trigger.restart();
                                }
                            }
                            _ => {}
                        }
                    }
                } else {
                    if let Ok(index) = res {
                        trigger.matching.remove(index);
                        match trigger.trigger_type {
                            AlarmTriggerType::WhileAnyActive => {
                                if trigger.matching.is_empty() {
                                    trigger.stop();
                                }
                            }
                            AlarmTriggerType::WhileNoneActive => {
                                if trigger.matching.is_empty() {
                                    trigger.start();
                                }
                            }
                            AlarmTriggerType::WhenCleared => {
                                trigger.restart();
                            }
                            AlarmTriggerType::WhenLastCleared => {
                                if trigger.matching.is_empty() {
                                    trigger.restart();
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

pub fn setup_alarms(
    player_conf: &PlayerConfig,
    playback_ctxt: &PlaybackContext,
    action_ctxt: &ActionContext,
) -> DynResult<AlarmContext> {
    let alarm_state = Vec::new();
    let mut alarm_triggers = Vec::new();

    let alarm_ctxt = AlarmContext {
        alarm_state,
        alarm_triggers,
    };
    Ok(alarm_ctxt)
}

struct StateMachineContext {
}

fn setup_state_machines(
    player_conf: &PlayerConfig,
    action_ctxt: &ActionContext,
) -> DynResult<StateMachineContext> {
    Ok(StateMachineContext {})
}
