use clap::{App, Arg, SubCommand};
use cpal::SampleFormat;
use log::error;
use mtp_audioplayer::util::error::DynResult;
use mtp_audioplayer::{
    app_config, clip_player::ClipPlayer, read_config, read_config::PlayerConfig,
    sample_buffer::SampleBuffer,
};
use std::path::Path;
use std::sync::Arc;

/*
fn default_volume() -> f64
{
    1.0
}

fn default_clip_root() -> String
{
    "".to_string()
}



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
    let app_args = App::new("Clip player")
        .version("0.1")
        .about("Test tools for clip playback")
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .value_name("FILE")
                .help("Load configuration file")
                .takes_value(true),
        )
        .subcommand(
            SubCommand::with_name("playfile")
                .about("Play a sound file")
                .arg(
                    Arg::with_name("FILE")
                        .help("A WAV-file to play")
                        .required(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("playclip")
                .about("Play a sound clip")
                .arg(
                    Arg::with_name("CLIP")
                        .help("Name of the clip to play")
                        .required(true)
                        .multiple(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("action")
                .about("Run a named action")
                .arg(
                    Arg::with_name("ACTION")
                        .help("Name of the action to run")
                        .required(true)
                        .multiple(false),
                ),
        )
        .subcommand(
            SubCommand::with_name("toggle_tag")
                .about("Run the toggle action for the tag")
                .arg(
                    Arg::with_name("TAG")
                        .help("Name of the tag to toggle")
                        .required(true)
                        .multiple(false),
                ),
        );

    let args = app_args.get_matches();

    let app_config;
    let base_dir;
    if let Some(conf_file) = args.value_of("config") {
        match read_config::read_file(conf_file) {
            Ok(conf) => app_config = Some(conf),
            Err(e) => {
                error!("Failed to read configuration file '{}': {}", conf_file, e);
                return;
            }
        }
        base_dir = Some(Path::new(conf_file).parent().unwrap());
    } else {
        app_config = None;
        base_dir = None;
    };

    match args.subcommand() {
        ("playfile", Some(args)) => {
            if let Some(file) = args.value_of("FILE") {
                if let Err(e) = play_file(file).await {
                    error!("{}", e);
                }
            }
        }
        ("playclip", Some(args)) => {
            let app_conf = match app_config {
                Some(c) => c,
                None => {
                    error!("No configuration");
                    return;
                }
            };
            if let Some(clips) = args.values_of("CLIP") {
                for clip in clips {
                    if let Err(e) = play_clip(&app_conf, clip, base_dir.unwrap()).await {
                        error!("{}", e);
                        return;
                    }
                }
            }
        }
        _ => {}
    }
}

async fn play_file(sound_file: &str) -> DynResult<()> {
    let mut samples;
    println!("File: {:?}", sound_file);
    match hound::WavReader::open(&sound_file) {
        Ok(mut reader) => {
            samples = Vec::<i16>::new();
            for s in reader.samples::<i16>() {
                match s {
                    Ok(s) => samples.push(s),
                    Err(err) => {
                        return Err(format!(
                            "Failed to read samples from file \"{}\": {}",
                            sound_file, err
                        )
                        .into())
                    }
                }
            }
        }
        Err(err) => {
            return Err(format!("Failed to open audio file \"{}\": {}", sound_file, err).into());
        }
    }
    let clip_player = match ClipPlayer::new("default", 44100, 2, SampleFormat::I16) {
        Err(e) => return Err(format!("Failed to initialise playback: {}", e).into()),
        Ok(c) => c,
    };

    let samples = Arc::new(SampleBuffer::I16(samples));
    clip_player.start_clip(samples.clone()).await?;
    clip_player.shutdown();
    Ok(())
}

async fn play_clip(app_conf: &PlayerConfig, clip: &str, base_dir: &Path) -> DynResult<()> {
    let playback_ctxt = app_config::setup_clip_playback(app_conf, base_dir)?;
    playback_ctxt.play(clip, 0).await?;
    Ok(())
}
