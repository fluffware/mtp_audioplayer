use crate::actions::action::Action;
use crate::actions::{
    parallel::ParallelAction, play::PlayAction, repeat::RepeatAction, sequence::SequenceAction,
    wait::WaitAction,
};
use crate::clip_queue::ClipQueue;
use crate::read_config::ActionType;
use crate::read_config::TagTriggerType;
use crate::{
    clip_player::ClipPlayer,
    read_config::{ClipType, PlayerConfig},
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
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
                let scale = amplitude * SAMPLE_MAX;
                let length = (rate * duration.as_secs_f64()).round() as usize;
                let mut samples = Vec::<i16>::with_capacity(length * usize::from(channels));
                let fscale = frequency * std::f64::consts::TAU / rate;
                for i in 0..length {
                    for _ in 0..channels {
                        samples.push((f64::sin((i as f64) * fscale) * scale) as i16);
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
    action_conf: &ActionType,
    action_map: &HashMap<String, Arc<dyn Action + Send + Sync>>,
) -> DynResult<Arc<dyn Action + Send + Sync>> {
    match action_conf {
        ActionType::Sequence(conf_actions) => {
            let mut sequence = SequenceAction::new();
            for conf_action in conf_actions {
                let action = action_conf_to_action(playback_ctxt, conf_action, action_map)?;
                sequence.add_arc_action(action);
            }
            Ok(Arc::new(sequence))
        }
        ActionType::Parallel(conf_actions) => {
            let mut parallel = ParallelAction::new();
            for conf_action in conf_actions {
                let action = action_conf_to_action(playback_ctxt, conf_action, action_map)?;
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
            let repeated = action_conf_to_action(playback_ctxt, action, action_map)?;
            Ok(Arc::new(RepeatAction::new(repeated, *count)))
        }
        ActionType::AlarmRestart => Err("Alarm restart action not implemented".into()),
        ActionType::SetProfile { profile: _ } => Err("Set profile action not implemented".into()),
    }
}

pub fn setup_actions(
    player_conf: &PlayerConfig,
    playback_ctxt: &PlaybackContext,
) -> DynResult<ActionContext> {
    let mut actions = HashMap::new();
    for (name, action_conf) in &player_conf.named_actions {
        let action = action_conf_to_action(playback_ctxt, action_conf, &actions)?;
        actions.insert(name.clone(), action);
    }
    Ok(ActionContext { actions })
}

pub struct TagTrigger {
    pub trigger: TagTriggerType,
    pub action: Arc<dyn Action + Send + Sync>,
}

pub trait TagObserver {
    // Called whenever a tag change value. Returns false if the function
    // should not be called any more.
    fn tag_changed(&mut self, name: &str, old_value: &Option<&str>, new_value: &str) -> bool;
}

pub struct TagContext {
    tag_state: HashMap<String, String>,
    tag_observers: HashMap<String, Vec<Box<dyn TagObserver>>>,
}

impl TagContext {
    pub fn new() -> TagContext {
        TagContext {
            tag_state: HashMap::new(),
            tag_observers: HashMap::new(),
        }
    }

    pub fn add_observer(&mut self, tag: String, obs: Box<dyn TagObserver>) {
        if let Some(observers) = self.tag_observers.get_mut(&tag) {
            observers.push(obs);
        } else {
            self.tag_observers.insert(tag, vec![obs]);
        }
    }

    pub fn tag_changed(&mut self, name: &str, new_value: &str) {
        if let Some(observers) = self.tag_observers.get_mut(name) {
            let old_value = self.tag_state.get(name);
	    let mut i = 0;
	    while i < observers.len() {
		let observer = &mut observers[i];
                if observer.tag_changed(name, &old_value.and_then(|s| Some(s.as_str())), new_value) {
		    i += 1;
		} else {
		    observers.remove(i);
		}
	    }
        }
	self.tag_state.insert(name.to_string(), new_value.to_string());
    }

    pub fn observed_tags<'a>(&'a self) -> impl Iterator<Item=&'a String>
    {
	self.tag_observers.keys()
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

struct ToggleObserver {
    action: Arc<dyn Action + Send + Sync>,
    cancel: Option<CancellationToken>,
}

impl ToggleObserver {
    pub fn new(action: Arc<dyn Action + Send + Sync>) -> ToggleObserver
    {
	ToggleObserver{
	    action,
	    cancel: None
	}
    }
}

impl TagObserver for ToggleObserver {
    fn tag_changed(&mut self, _name: &str, old_value: &Option<&str>, new_value: &str) -> bool {
        if let Some(old_value) = old_value {
            if bool_value(old_value) != bool_value(new_value) {
		if let Some(cancel) = self.cancel.take() {
		    cancel.cancel();
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
            }
        }
        true
    }
}

impl Drop for ToggleObserver
{
    fn drop(&mut self) {
	if let Some(cancel) = self.cancel.take() {
	    cancel.cancel();
	}
    }
}

struct WhileEqualObserver {
    action: Arc<dyn Action + Send + Sync>,
    cancel: Option<CancellationToken>,
    equals: i32,
}

impl WhileEqualObserver {
    pub fn new(action: Arc<dyn Action + Send + Sync>, equals: i32) -> WhileEqualObserver
    {
	WhileEqualObserver{
	    action,
	    cancel: None,
            equals,
	}
    }
}

impl TagObserver for WhileEqualObserver {
    fn tag_changed(&mut self, _name: &str, old_value: &Option<&str>, new_value: &str) -> bool {
        if let Ok(new_value) = new_value.parse::<i32>() {
            if let Some(old_value) = old_value.and_then(|v| v.parse::<i32>().ok()) {
                if old_value != new_value {
		    if let Some(cancel) = self.cancel.take() {
		        cancel.cancel();
		    }
                    if new_value == self.equals {
                        let action = self.action.clone();
		        let cancel = CancellationToken::new();
		        self.cancel = Some(cancel.clone());
                        tokio::spawn(async move {
                            tokio::select! {
                                _ = action.run() => {},
                                _ = cancel.cancelled() => {},
                            }
                        });
                    }
                }
            }
        }
        true
    }
}

impl Drop for WhileEqualObserver
{
    fn drop(&mut self) {
	if let Some(cancel) = self.cancel.take() {
	    cancel.cancel();
	}
    }
}
pub fn setup_tags(
    player_conf: &PlayerConfig,
    playback_ctxt: &PlaybackContext,
    action_ctxt: &ActionContext,
) -> DynResult<TagContext> {
    let mut tag_ctxt = TagContext::new();
    for (name, trigger_conf) in &player_conf.tag_triggers {
        let action =
            action_conf_to_action(playback_ctxt, &trigger_conf.action, &action_ctxt.actions)?;
	match trigger_conf.trigger {
	    TagTriggerType::Toggle => {
		tag_ctxt.add_observer(name.to_string(), Box::new(ToggleObserver::new(action)));
	    },
            TagTriggerType::Equals{value} => {
		tag_ctxt.add_observer(name.to_string(), Box::new(WhileEqualObserver::new(action, value)));
            },
	    //_ => {}
	}
    }
    Ok(tag_ctxt)
}
