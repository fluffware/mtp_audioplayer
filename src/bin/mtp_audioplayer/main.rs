use clap::{Arg, Command};
use git_version::git_version;
use log::{debug, error, warn};
use mtp_audioplayer::actions::tag_setter::TagSetter;
use mtp_audioplayer::app_config::{
    self, AlarmContext, StateMachineContext, TagContext, TagSetRequest, VolumeControlContext,
};
use mtp_audioplayer::daemon;
use mtp_audioplayer::open_pipe::alarm_data::AlarmData;
use mtp_audioplayer::open_pipe::connection as open_pipe;
use mtp_audioplayer::read_config::{self, PlayerConfig};
use mtp_audioplayer::util::error::DynResult;
use open_pipe::{MessageVariant, WriteTagValue};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;
use tokio::signal;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::{timeout, Duration};

const DEFAULT_CONFIG_FILE: &str = "mtp_audioplayer.xml";

async fn subscribe_tags(
    pipe: &mut open_pipe::Connection,
    tag_names: &mut [String],
) -> DynResult<(String, HashMap<String, String>)> {
    let mut tag_values = HashMap::<String, String>::new();

    let value_tags: Vec<&str> = tag_names.iter().map(|c| c.as_str()).collect();
    debug!("Subcribing: {:?}", value_tags);
    let subscription = pipe.subscribe_tags(&value_tags).await?;

    'next_event: loop {
        match timeout(Duration::from_secs(1), pipe.get_message()).await {
            Err(_) => {
                return Err("No reply for tag subscription".to_string().into());
            }
            Ok(res) => match res {
                Ok(event) => {
                    if let MessageVariant::NotifySubscribeTag(params) = event.message {
                        for tag in params.params.tags {
                            if tag.error.error_code == 0 {
                                tag_values.insert(tag.data.name, tag.data.value);
                            } else {
                                warn!("Failed to subscribe to {}", tag.data.name);
                            }
                        }
                        break 'next_event;
                    }
                }
                Err(e) => return Err(e),
            },
        }
    }
    Ok((subscription, tag_values))
}
async fn subscribe_alarms(pipe: &mut open_pipe::Connection) -> DynResult<Vec<AlarmData>> {
    debug!("Subcribing alarms");
    let _subscription = pipe.subscribe_alarms().await?;
    let alarms;
    'next_event: loop {
        match timeout(Duration::from_secs(5), pipe.get_message()).await {
            Err(_) => {
                return Err("No reply for alarm subscription".to_string().into());
            }
            Ok(res) => match res {
                Ok(event) => match event.message {
                    MessageVariant::NotifySubscribeAlarm(params) => {
                        debug!("Subcribed alarms: {:?}", params);
                        alarms = params
                            .params
                            .alarms
                            .into_iter()
                            .map(AlarmData::from)
                            .collect();
                        break 'next_event;
                    }
                    MessageVariant::ErrorSubscribeAlarm(error) => return Err(error.into()),
                    _ => {}
                },
                Err(e) => {
                    return Err(e);
                }
            },
        }
    }
    Ok(alarms)
}

fn trig_on_tag(tag_ctxt: &Arc<TagContext>, tag_name: &str, tag_value: &str) {
    tag_ctxt.tag_changed(tag_name, tag_value);
}

type ConfigurationResult = DynResult<(
    PlayerConfig,
    Arc<TagContext>,
    Arc<AlarmContext>,
    Arc<VolumeControlContext>,
    StateMachineContext,
    UnboundedReceiver<TagSetRequest>,
)>;

fn read_configuration(path: &Path) -> ConfigurationResult {
    let app_conf = read_config::read_file(path)?;
    let base_dir = Path::new(path)
        .parent()
        .ok_or("Configuration file has no parent")?;

    let (pipe_send_tx, pipe_send_rx) = tokio::sync::mpsc::unbounded_channel::<TagSetRequest>();
    let playback_ctxt = app_config::setup_clip_playback(&app_conf, base_dir)?;
    let volume_ctxt = Arc::new(app_config::setup_volume_control(&app_conf)?);
    let tag_ctxt = app_config::setup_tags(&app_conf, pipe_send_tx)?;
    let tag_ctxt = Arc::new(tag_ctxt);
    let alarm_ctxt = app_config::setup_alarms(&app_conf, Arc::downgrade(&tag_ctxt))?;
    let alarm_ctxt = Arc::new(alarm_ctxt);
    let state_machine_ctxt = app_config::setup_state_machines(
        &app_conf,
        &playback_ctxt,
        &tag_ctxt,
        &volume_ctxt,
        &alarm_ctxt,
    )?;
    Ok((
        app_conf,
        tag_ctxt,
        alarm_ctxt,
        volume_ctxt,
        state_machine_ctxt,
        pipe_send_rx,
    ))
}

type MessageHandler = Box<dyn FnMut(&open_pipe::Message) -> DynResult<bool>>;

#[tokio::main]
async fn main() {
    let version = env!("CARGO_PKG_VERSION").to_string() + " " + git_version!();
    let app_args = Command::new("MTP audio player")
        .version(version.as_str())
        .about("Server for audio playback on Siemens Unified Comfort HMI panels")
        .arg(
            Arg::new("CONF")
                .default_value(DEFAULT_CONFIG_FILE)
                .help("Configuration file"),
        );

    let app_args = daemon::add_args(app_args);
    let args = app_args.get_matches();

    let conf_path_str = OsStr::new(args.value_of("CONF").unwrap());

    daemon::start(&args);

    let (app_conf, tag_ctxt, alarm_ctxt, _volume_ctxt, state_machine_ctxt, mut pipe_send_rx) =
        match read_configuration(Path::new(&conf_path_str)) {
            Ok(ctxt) => ctxt,
            Err(e) => {
                error!(
                    "Failed to read configuration file '{}': {}",
                    conf_path_str.to_string_lossy(),
                    e
                );
                return;
            }
        };
    tag_ctxt.add_tag("AUDIO_SERVER_VERSION", None);
    let mut pipe = match open_pipe::Connection::connect(&app_conf.bind).await {
        Err(err) => {
            error!("Failed open connection to {}: {}", app_conf.bind, err);
            return;
        }
        Ok(c) => c,
    };

    let running_sm = state_machine_ctxt.run_all();
    tokio::pin!(running_sm);

    let mut tag_names: Vec<String> = tag_ctxt.tag_names();
    match subscribe_tags(&mut pipe, &mut tag_names).await {
        Err(e) => {
            error!("Failed to subscribe tags: {}", e);
            return;
        }
        Ok((_, mut values)) => {
            for (k, v) in values.drain() {
                tag_ctxt.tag_changed(&k, &v);
            }
        }
    }

    if tag_names.is_empty() {
        error!("No tags subscribed");
        return;
    }

    match subscribe_alarms(&mut pipe).await {
        Err(e) => {
            error!("Failed to subscribe alarms: {}", e);
            return;
        }
        Ok(alarms) => {
            for alarm_data in alarms {
                if let Err(e) = alarm_ctxt.handle_notification(&alarm_data) {
                    error!("Failed to handle alarm notification: {}", e);
                }
            }
        }
    }

    let mut handler_list = Vec::<MessageHandler>::new();

    // Handle NotifySubscribeTag message
    handler_list.push(Box::new(|msg: &open_pipe::Message| {
        if let MessageVariant::NotifySubscribeTag(notify) = &msg.message {
            for notify_tag in &notify.params.tags {
                trig_on_tag(&tag_ctxt, &notify_tag.data.name, &notify_tag.data.value);
            }
        }
        Ok(true)
    }));

    // Handle NotifySubscribeAlarm message
    handler_list.push(Box::new(|msg: &open_pipe::Message| {
        if let MessageVariant::NotifySubscribeAlarm(notify) = &msg.message {
            for notify_alarm in &notify.params.alarms {
                debug!("Received alarm: {:?}", notify_alarm);
                let alarm_data = AlarmData::from(notify_alarm.clone());
                if let Err(e) = alarm_ctxt.handle_notification(&alarm_data) {
                    error!("Failed to handle alarm notification: {}", e);
                }
            }
        }
        Ok(true)
    }));
    tag_ctxt.set_tag("AUDIO_SERVER_VERSION", version.as_str());
    daemon::ready();
    let mut done = false;
    while !done {
        tokio::select! {
            res = signal::ctrl_c() => {
                if let Err(e) = res {
                    error!("Failed to wait for ctrl-c: {}",e);
                }
                done = true;
            },
            res = pipe_send_rx.recv() => {
                if let  Some(req) = res {
                    let write_tag = WriteTagValue {
                        name: req.tag_name.clone(),
                            value: req.value
                    };
                    if let Err(e) = pipe.write_tags(&[write_tag]).await {
                        error!("Failed to write tag to pipe: {}",e);
                    }
                    let mut done = Some(req.done);
                    let name = req.tag_name;
                    // Queue a handler that waits for the write to be confirmed
                    handler_list.push(Box::new(move |msg: &open_pipe::Message| {
                        if let MessageVariant::NotifyWriteTag(notify) = &msg.message {
                            for tag in &notify.params.tags {
                                if tag.name == name {
                                    let _ = done.take().unwrap().send(Ok(()));
                                        return Ok(false)
                                }
                            }
                            Ok(true)
                            } else {
                            Ok(!done.as_ref().unwrap().is_closed())
                        }
                        }
                    ));
                }
            },
            res = pipe.get_message() => {
                match res {
                    Err(_) => {
                        done = true
                    },
                    Ok(msg) => {
                        let mut i = 0;
                        while i < handler_list.len() {
                            match handler_list[i](&msg) {
                                Ok(again) => {
                                    if again {
                                        i += 1;
                                    } else {
                                        let _ = handler_list.remove(i);
                                    }
                                },
                                Err(e) => {
                                    error!("Failed to handle Open Pipe message: {}",e);
                                    return;
                                }
                            }
                        }
                    }
                }
            }

            res = &mut running_sm => {
                match res {
                    Ok(_) => {
                        error!("State machine stopped");
                        done = true;
                    }
                    Err(err) => {
                        error!("State machine error: {}", err);
                        return;
                    }
                }
            }
        }
    }

    daemon::exiting();
}
