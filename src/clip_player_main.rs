use serde::Deserialize;
use serde_json;
use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::fs::File;
use tokio::signal;
use log::{error,warn,debug};
use std::sync::Arc;
use std::collections::BTreeMap;

mod clip_player;
use clip_player::ClipPlayer;

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
    _bind: String,
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
const DEFAULT_CONFIG_FILE: &'static str = "mtp_audioplayer.conf";

struct ClipData
{
    samples: Arc<Vec<i16>>,
    volume: f64
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
                                      volume: c.volume});
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

    let clip_name =   if let Some(name) = args.next() {
        name.to_string_lossy().into_owned()
    } else {
        error!("No clip name given");
        return;
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

    if let Some(clip) = clip_map.get(&*clip_name) {
        debug!("Last samples: {}", clip.samples[(clip.samples.len() - 10)..].iter().map(|s| s.to_string()).collect::<Vec<String>>().join(", "));
        clip_player.start_clip(clip.samples.clone()).unwrap();
    } else {
        error!("No clip named {} found", clip_name);
        return;
    }
    
    let mut done = false;
    while !done {
        tokio::select! {
            res = signal::ctrl_c() => {
                if let Err(e) = res {
                    error!("Failed to wait for ctrl-c: {}",e);
                }
                done = true;
            }
        }
    }
}
