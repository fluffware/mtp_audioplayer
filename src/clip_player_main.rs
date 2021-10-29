use std::env;
use log::{error,warn};
use std::sync::Arc;
use tokio::time::{timeout, Duration};

use mtp_audioplayer::clip_player::ClipPlayer;

/*
fn default_volume() -> f64
{
    1.0
}

fn default_clip_root() -> String
{
    "".to_string()
}


type Result<T> = 
    std::result::Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;

    
const SAMPLE_MAX:f64 = std::i16::MAX as f64;
const SAMPLE_MIN:f64 = std::i16::MIN as f64;

fn adjust_volume(volume: f64, buffer: &mut [i16])
{
    for s in buffer {
        *s = ((*s as f64) * volume).max(SAMPLE_MIN).min(SAMPLE_MAX).round() as i16;
    }
}
*/

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let args = env::args_os();
    let mut args = args.skip(1);
    let sound_file = if let Some(path) = args.next() {
        path
    } else {
        error!("No sound file");
        return;
    };

    let mut samples;
    println!("File: {:?}", sound_file);
    match hound::WavReader::open(&sound_file) {
        Ok(mut reader) => {
            samples = Vec::<i16>::new();
            for s in reader.samples::<i16>() {
                match s {
                    Ok(s) => samples.push(s),
                    Err(err) => {
                        warn!("Failed to read samples from file \"{}\": {}",
                              sound_file.to_string_lossy(), err);
                        break;
                    }
                }
            }
        },
        Err(err) => {
            warn!("Failed to open audio file \"{}\": {}",
                  sound_file.to_string_lossy(), err);
            return;
        }
    }
    let clip_player = match ClipPlayer::new("default",
                                            41000, 2) {
        Err(e) => {
            error!("Failed to initialise playback: {}",e);
            return;
        },
        Ok(c) => c
    };

    let samples = Arc::new(samples);
    for _ in 0..2 {
        clip_player.start_clip(samples.clone()).await.unwrap();
    }
    timeout(Duration::from_millis(500),
            clip_player.start_clip(samples.clone())).await.unwrap().unwrap();
    clip_player.start_clip(samples.clone()).await.unwrap();
    clip_player.shutdown();
}
