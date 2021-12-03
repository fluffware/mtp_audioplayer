use std::collections::HashMap;
use std::sync::Arc;
use std::path::{Path};
use crate::clip_queue::ClipQueue;
use crate::actions::action::Action;
use crate::read_config::ActionType;
use crate::read_config::TagTriggerType;
use crate::actions::{
    sequence::SequenceAction,
    parallel::ParallelAction,
    play::PlayAction,
    wait::WaitAction,
    repeat::RepeatAction
};

use crate::{
    clip_player::ClipPlayer,
    read_config::{
	ClipType,
	PlayerConfig
    }
};
type DynResult<T> = 
    std::result::Result<T, Box<dyn std::error::Error + Send +Sync>>;

fn load_clip(os_file: &Path) -> DynResult<Arc<Vec<i16>>>
{
    let mut samples;
    match hound::WavReader::open(&os_file) {
        Ok(mut reader) => {
	    samples = Vec::<i16>::new();
            for s in reader.samples::<i16>() {
                match s {
                    Ok(s) => samples.push(s),
                    Err(err) => 
                        return Err(format!(
			    "Failed to read samples from file \"{}\": {}",
			    os_file.to_string_lossy(), err).into())
		}
            }
        },
        Err(err) => {
            return Err(format!("Failed to open audio file \"{}\": {}",
			       os_file.to_string_lossy(), err).into())
        }
    }
    Ok(Arc::new(samples))
}

const SAMPLE_MAX:f64 = std::i16::MAX as f64;

pub fn load_clips(clip_root: &Path, clip_conf: &HashMap<String,ClipType>,
		  rate: u32, channels: u8)
		  -> DynResult<HashMap<String, Arc<Vec<i16>>>>
{
    let mut clips = HashMap::<String, Arc<Vec<i16>>>::new();
    for (name,conf) in clip_conf {
	match conf {
	    ClipType::File(f) => {
		let os_name = clip_root.join(f);
		let samples = load_clip(&os_name)?;
		clips.insert(name.clone(), samples);
	    },
	    ClipType::Sine{amplitude, frequency, duration} => {
		let rate = f64::from(rate);
		let scale = amplitude * SAMPLE_MAX;
		let length = (rate * duration.as_secs_f64()).round() as usize;
		let mut samples = Vec::<i16>::with_capacity(
		    length*usize::from(channels));
		let fscale = frequency * std::f64::consts::TAU / rate;
		for i in 0..length {
		    for _ in 0..channels {
			samples.push((f64::sin((i as f64) * fscale) 
				      * scale) as i16);
		    }
		}
		
		clips.insert(name.clone(), Arc::new(samples));
	    }
	}
    }
    Ok(clips)
}

#[derive(Debug)]
pub enum PlaybackError
{
    NameNotFound(String),
}

impl std::error::Error for PlaybackError {}

impl std::fmt::Display for PlaybackError
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>)
           -> std::result::Result<(), std::fmt::Error>
    {
        match self {
	    Self::NameNotFound(name) =>
		write!(f, "Clip '{}' not found", name)
	}
    }
}


pub struct PlaybackContext
{
    pub rate: u32,
    pub channels: u8,
    pub clip_queue: Arc<ClipQueue>,
    pub clips: HashMap<String, Arc<Vec<i16>>>
}

impl PlaybackContext
{
    pub async fn play(&self, clip_name: &str, priority: i32) -> DynResult<()>
    {
	let clip = self.clips.get(clip_name).ok_or_else(
	    || PlaybackError::NameNotFound(clip_name.to_string()))?;
	self.clip_queue.play(clip.clone(), priority, None).await?;
	
	Ok(())
    }
}

pub fn setup_clip_playback(player_conf: &PlayerConfig, base_dir: &Path)
		     -> DynResult<PlaybackContext>
{
    let clip_root = base_dir.join(&player_conf.clip_root);
    let clips = load_clips(&clip_root, &player_conf.clips, player_conf.rate, player_conf.channels)?;
    let rate = player_conf.rate;
    let channels = player_conf.channels;
    let clip_player = ClipPlayer::new(&player_conf.playback_device,
                                      rate, channels).map_err(
	|e| format!("Failed to initialise playback: {}",e))?;

    let clip_queue = ClipQueue::new(clip_player);
    Ok(PlaybackContext{
	rate,
	channels,
	clip_queue: Arc::new(clip_queue),
	clips
    })
}

pub struct ActionContext
{
    pub actions: HashMap<String, Arc<dyn Action>>
}

fn action_conf_to_action(playback_ctxt: &PlaybackContext,
			 action_conf: &ActionType,
			 action_map: &HashMap<String, Arc<dyn Action>>) 
			 -> DynResult<Arc<dyn Action>>
{
    match action_conf {
	ActionType::Sequence(conf_actions) => {
	    let mut sequence = SequenceAction::new();
	    for conf_action in conf_actions {
		let action = action_conf_to_action(playback_ctxt, 
						   conf_action, action_map)?;
		sequence.add_arc_action(action);
	    }
	    Ok(Arc::new(sequence))
	},
	ActionType::Parallel(conf_actions) => {
	    let mut parallel = ParallelAction::new();
	    for conf_action in conf_actions {
		let action = action_conf_to_action(playback_ctxt, 
						   conf_action, action_map)?;
		parallel.add_arc_action(action);
	    }
	    Ok(Arc::new(parallel))
	},
	ActionType::Play{priority, timeout, sound} => {
	    let samples = playback_ctxt.clips.get(sound).ok_or_else(
		|| format!("No clip named '{}'", sound))?;
	    let action = PlayAction::new(playback_ctxt.clip_queue.clone(),
					 *priority,
					 *timeout,
					 samples.clone());
	    Ok(Arc::new(action))
					 
	},
	ActionType::Wait(timeout) => {
	    Ok(Arc::new(WaitAction::new(*timeout)))
	},
	ActionType::Reference(action_ref) => {
	    let action = action_map.get(action_ref).ok_or_else(
		|| format!("No preceding action with id {} found", action_ref)
	    )?;
	    
	    Ok(action.clone())
	},
	ActionType::Repeat{count, action} => {
	    let repeated = action_conf_to_action(playback_ctxt, action, 
						 action_map)?;
	    Ok(Arc::new(RepeatAction::new(repeated, *count)))
	},
	ActionType::AlarmRestart => {
	    Err("Alarm restart action not implemented".into())
	},
	ActionType::SetProfile{profile: _} => {
	     Err("Set profile action not implemented".into())
	}
    }
}

pub fn setup_actions(player_conf: &PlayerConfig, 
		     playback_ctxt: &PlaybackContext)
		     -> DynResult<ActionContext>
{
    let mut actions = HashMap::new();
    for (name, action_conf) in &player_conf.named_actions {
	let action = action_conf_to_action(playback_ctxt, action_conf, &actions)?;
	actions.insert(name.clone(), action);
    }
    Ok(ActionContext{actions})
}

pub struct TagTrigger
{
    pub trigger: TagTriggerType,
    pub action: Arc<dyn Action>
}

pub struct TagContext
{
    pub triggers: HashMap<String, Vec<TagTrigger>>
}

pub fn setup_tags(player_conf: &PlayerConfig,
		  playback_ctxt: &PlaybackContext,
		  action_ctxt: &ActionContext)
		  -> DynResult<TagContext>
{
    let mut triggers = HashMap::<String,Vec<TagTrigger>>::new();
    for (name, trigger_conf) in &player_conf.tag_triggers {
	let action = action_conf_to_action(playback_ctxt, &
					   trigger_conf.action,
					   &action_ctxt.actions)?;
	let trigger = TagTrigger{trigger: trigger_conf.trigger.clone(),
				 action};
	if let Some(triggers) = triggers.get_mut(name) {
	    triggers.push(trigger);
	} else {
	    triggers.insert(name.clone(), vec![trigger]);
	}
    }
    Ok(TagContext{triggers}) 
}
