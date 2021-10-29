use serde::Deserialize;
use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::fs::File;
use tokio::signal;
use tokio::time::{timeout, Duration};
use log::{error,debug,warn};
use std::sync::Arc;
use std::collections::BTreeMap;


use mtp_audioplayer::open_pipe::{
    self,
    MessageVariant,
    WriteTagValue
};

use mtp_audioplayer::clip_player::ClipPlayer;


fn default_volume() -> f64
{
    1.0
}

fn default_clip_root() -> String
{
    "".to_string()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipConfig
{
    tag: String,
    file: String,
    #[serde(default="default_volume")]
    volume: f64
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config
{
    playback_device: String,
    bind: String,
    rate: u32,
    channels: u8,
    clips: Vec<ClipConfig>,
    #[serde(default="default_clip_root")]
    clip_root: String
}

type Result<T> = 
    std::result::Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;

fn read_config(path: &Path) 
               -> Result<Config>
{
    /*
    let conf = Config {
        bind: "/tmp/siemens/automation/HmiRuntime".to_string(),
        rate: 44100,
        channels: 2,
        clips: Vec::new()
    };*/
    let f = File::open(path)?;
    let conf : Config = serde_json::from_reader(f)?;
    Ok(conf)
}
const DEFAULT_CONFIG_FILE: &str = "mtp_audioplayer.conf";


async fn subscribe_tags(pipe: &mut open_pipe::Connection,
                        tag_names: &mut Vec<String>)
                        -> Result<()>
{
    'try_subscribe:
    loop {
        let value_tags: Vec<&str> = 
            tag_names.iter().map(|c| c.as_str()).collect();
        debug!("Subcribing: {:?}", value_tags);
        let subscription = pipe.subscribe_tags(&value_tags).await?;
        let mut resubscribe = false;// Retry subscription until all tags succeed
        'next_event:
        loop {
            match timeout(Duration::from_secs(1), pipe.get_event()).await {
            Err(_) => {
                return Err("No reply for tag subscription".to_string().into());
            },
                Ok(res) => match res {
                    Some(event) => {
                        if let MessageVariant::NotifySubscribeTag(params) 
                        = event.message {
                            tag_names.clear();
                            for tag in params.params.tags {
                                if tag.error.error_code == 0 {
                                    tag_names.push(tag.name);
                                } else {
                                    warn!("Failed to subscribe to {}", tag.name);
                                    resubscribe = true;
                                }
                            }
                        }
                        if resubscribe {
                            pipe.unsubscribe_tags(&subscription).await?;
                            continue 'next_event;
                        } else {
                            break 'try_subscribe;
                        }
                    },
                    None => {
                        error!("Message EOF");
                        return Err("Message EOF".to_string().into());
                    }
                }
            }
        }
    }
    Ok(())
}
async fn subscribe_alarms(pipe: &mut open_pipe::Connection) -> Result<()>
{
    debug!("Subcribing alarms");
    let _subscription = pipe.subscribe_alarms().await?;
    'next_event:
    loop {
        match timeout(Duration::from_secs(5), pipe.get_event()).await {
            Err(_) => {
                return Err("No reply for alarm subscription".to_string().into());
            },
            Ok(res) => match res {
                Some(event) => {
                    match event.message {
                        MessageVariant::NotifySubscribeAlarm(params) =>
                        {
                            debug!("Subcribed alarms: {:?}", params);
                            break 'next_event;
                        },
                        MessageVariant::ErrorSubscribeAlarm(error) => {
                            return Err(error.into())
                        },
                        _ => {}
                    }
                },
                None => {
                    error!("Message EOF");
                    return Err("Message EOF".to_string().into());
                }
            }
        }
    }
    Ok(())
}
struct ClipData
{
    samples: Arc<Vec<i16>>,
    _volume: f64
}
    
const SAMPLE_MAX:f64 = std::i16::MAX as f64;
const SAMPLE_MIN:f64 = std::i16::MIN as f64;

fn adjust_volume(volume: f64, buffer: &mut [i16])
{
    for s in buffer {
        *s = ((*s as f64) * volume).max(SAMPLE_MIN).min(SAMPLE_MAX).round() as i16;
    }
}
fn read_clips(file_root: &Path, clip_conf: &[ClipConfig]) -> BTreeMap<String, ClipData>
{
    let mut clips = BTreeMap::new();
    for c in clip_conf {
        let mut samples;
        let mut path = PathBuf::from(file_root);
        path.push(&c.file);
        match hound::WavReader::open(&path) {
            Ok(mut reader) => {
                samples = Vec::<i16>::new();
                for s in reader.samples::<i16>() {
                    match s {
                        Ok(s) => samples.push(s),
                        Err(err) => {
                            warn!("Failed to read samples from file \"{}\": {}",
                                  path.to_string_lossy(), err);
                            break;
                        }
                    }
                };
                adjust_volume(c.volume, &mut samples);
                clips.insert(c.tag.clone(),
                             ClipData{samples:Arc::new(samples),
                                      _volume: c.volume});
            },
            Err(err) => {
                warn!("Failed to open audio file \"{}\": {}",
                           path.to_string_lossy(), err);
                continue;
            }
        }
    }
    clips
}

async fn handle_msg(pipe: &mut open_pipe::Connection, 
                    msg: &open_pipe::Message,
                    player: &ClipPlayer,
                    clips: &BTreeMap<String,ClipData>) -> Result<()>
{
    let mut set_tags = Vec::<WriteTagValue>::new();
    match &msg.message {
        MessageVariant::NotifySubscribeTag(notify) => {
            for notify_tag in &notify.params.tags {
                if notify_tag.value.to_lowercase() == "true" {
                    set_tags.push(WriteTagValue{
                        name: notify_tag.name.clone(),
                        value: "FALSE".to_string()
                    });
                    if let Some(clip) = clips.get(&notify_tag.name) {
                        debug!("Playing {}", notify_tag.name);
                        player.start_clip(clip.samples.clone()).await?;
                    }
                }
            }
        },
        MessageVariant::NotifySubscribeAlarm(notify) => {
            for notify_alarm in &notify.params.alarms {
                debug!("Received alarm: {:?}", notify_alarm);
            }
        },
        _ => {}
    }
    if !set_tags.is_empty() {
        if let Err(e) = pipe.write_tags(&set_tags).await {
            error!("Failed to change tags: {}", e);
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let args = env::args_os();
    let mut args = args.skip(1);
    let conf_path_str = 
        if let Some(path) = args.next() {
            path
        } else {
            OsStr::new(DEFAULT_CONFIG_FILE).to_os_string()
        };
    let conf =
        match read_config(Path::new(&conf_path_str)) {
            Err(err) => {
                error!("Failed to read configuration file {}: {}", conf_path_str.to_string_lossy(), err);
                return
            },
            Ok(c) => c
        };
    
    let clip_player = match ClipPlayer::new(&conf.playback_device,
                                            conf.rate, conf.channels) {
        Err(e) => {
            error!("Failed to initialise playback: {}",e);
            return;
        },
        Ok(c) => c
    };

    let clip_root = if conf.clip_root.is_empty() {
        match Path::new(&conf_path_str).parent() {
            Some(p) => p.to_path_buf(),
            None => PathBuf::from(&conf.clip_root)
        }
    } else {
        PathBuf::from(&conf.clip_root)
    };

    let clip_map = read_clips(&clip_root, &conf.clips);
    let mut pipe = match open_pipe::Connection::connect(&conf.bind).await {
        Err(err) => {
             error!("Failed open connection to {}: {}", conf.bind, err);
            return
        },
        Ok(c) => c
    };
    let mut tag_names: Vec<String> = conf.clips.iter().map(|c| c.tag.clone()).collect();
    if let Err(e) = subscribe_tags(&mut pipe, &mut tag_names).await {
        error!("Failed to subscribe tags: {}",e);
        return;
    }

    if tag_names.is_empty() {
        error!("No tags subscribed");
        return;
    }
    
    if let Err(e) = subscribe_alarms(&mut pipe).await {
        error!("Failed to subscribe alarms: {}",e);
        return;
    }
    

    let tag_values = tag_names.iter().map(|t| {
        WriteTagValue{name: t.clone(), value: "FALSE".to_string()}
    }).collect::<Vec<WriteTagValue>>();
    if let Err(e) = pipe.write_tags(&tag_values).await {
        error!("Failed to clear tags: {}", e);
    }

    
    let mut done = false;
    while !done {
        tokio::select! {
            res = signal::ctrl_c() => {
                if let Err(e) = res {
                    error!("Failed to wait for ctrl-c: {}",e);
                }
                done = true;
            },
            res = pipe.get_event() => {
                match res {
                    None => {
                        done = true
                    },
                    Some(msg) => {
                        if let Err(e) =
                            handle_msg(&mut pipe, &msg, 
                                       &clip_player, &clip_map).await {
                                error!("Failed to handle Open Pipe message: {}",e);
                                return;
                            }
                    }
                }
            }
        }
    }
}
