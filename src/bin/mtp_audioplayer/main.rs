use log::{debug, error, info, warn};
use mtp_audioplayer::app_config::{self, AlarmContext, TagContext, StateMachineContext};
use mtp_audioplayer::open_pipe::alarm_data::AlarmData;
use mtp_audioplayer::open_pipe::connection as open_pipe;
use mtp_audioplayer::read_config::{self, PlayerConfig};
use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;
use tokio::signal;
use tokio::time::{timeout, Duration};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use open_pipe::{MessageVariant, WriteTagValue};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;

const DEFAULT_CONFIG_FILE: &str = "mtp_audioplayer.xml";

async fn subscribe_tags(
    pipe: &mut open_pipe::Connection,
    tag_names: &mut Vec<String>,
) -> Result<(String, HashMap<String, String>)> {
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
                Err(e) => return Err(e.into()),
            },
        }
    }
    Ok((subscription, tag_values))
}
async fn subscribe_alarms(pipe: &mut open_pipe::Connection) -> Result<Vec<AlarmData>> {
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
                            .map(|a| AlarmData::from(a))
                            .collect();
                        break 'next_event;
                    }
                    MessageVariant::ErrorSubscribeAlarm(error) => return Err(error.into()),
                    _ => {}
                },
                Err(e) => {
                    return Err(e.into());
                }
            },
        }
    }
    Ok(alarms)
}

async fn trig_on_tag(tag_ctxt: &Arc<TagContext>, tag_name: &str, tag_value: &str) {
    tag_ctxt.tag_changed(tag_name, tag_value);
}

async fn handle_msg(
    pipe: &mut open_pipe::Connection,
    msg: &open_pipe::Message,
    tag_ctxt: &Arc<TagContext>,
    alarm_ctxt: &Arc<AlarmContext>,
) -> Result<()> {
    let set_tags = Vec::<WriteTagValue>::new();
    match &msg.message {
        MessageVariant::NotifySubscribeTag(notify) => {
            for notify_tag in &notify.params.tags {
                trig_on_tag(tag_ctxt, &notify_tag.data.name, &notify_tag.data.value).await;
            }
        }
        MessageVariant::NotifySubscribeAlarm(notify) => {
            for notify_alarm in &notify.params.alarms {
                debug!("Received alarm: {:?}", notify_alarm);
                let alarm_data = AlarmData::from(notify_alarm.clone());
                if let Err(e) = alarm_ctxt.handle_notification(&alarm_data) {
                    error!("Failed to handle alarm notification: {}", e);
                }
            }
        }
        _ => {}
    }
    if !set_tags.is_empty() {
        if let Err(e) = pipe.write_tags(&set_tags).await {
            error!("Failed to change tags: {}", e);
        }
    }
    Ok(())
}

fn read_configuration(path: &Path) -> Result<(PlayerConfig, Arc<TagContext>, Arc<AlarmContext>, StateMachineContext, UnboundedReceiver<(String,String)>)> {
    let app_conf = read_config::read_file(path)?;
    let base_dir = Path::new(path)
        .parent()
        .ok_or("Configuration file has no parent")?;

     let (pipe_send_tx, mut pipe_send_rx) = tokio::sync::mpsc::unbounded_channel::<(String, String)>();
    let playback_ctxt = app_config::setup_clip_playback(&app_conf, base_dir)?;
    let tag_ctxt = app_config::setup_tags(&app_conf, pipe_send_tx)?;
    let tag_ctxt = Arc::new(tag_ctxt);
    let alarm_ctxt = app_config::setup_alarms(&app_conf)?;
    let alarm_ctxt = Arc::new(alarm_ctxt);
    let state_machine_ctxt = app_config::setup_state_machines(&app_conf, &playback_ctxt, &tag_ctxt, &alarm_ctxt)?;
    Ok((app_conf, tag_ctxt, alarm_ctxt, state_machine_ctxt, pipe_send_rx))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let args = env::args_os();
    let mut args = args.skip(1);
    let conf_path_str = if let Some(path) = args.next() {
        path
    } else {
        OsStr::new(DEFAULT_CONFIG_FILE).to_os_string()
    };

    let (app_conf, tag_ctxt, alarm_ctxt, state_machine_ctxt, mut pipe_send_rx) =
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

    let mut pipe = match open_pipe::Connection::connect(&app_conf.bind).await {
        Err(err) => {
            error!("Failed open connection to {}: {}", app_conf.bind, err);
            return;
        }
        Ok(c) => c,
    };

    state_machine_ctxt.start_all().await;
    
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
                match res {
                    Some((name, value)) => {
                        if let Err(e) = pipe.write_tags(&[WriteTagValue{name, value}]).await {
                            error!("Failed to write tag to pipe: {}",e);
                        }
                    }
                    None => {}
                }
            },
            res = pipe.get_message() => {
                match res {
                    Err(_) => {
                        done = true
                    },
                    Ok(msg) => {
                        if let Err(e) =
                            handle_msg(&mut pipe, &msg,
                                       &tag_ctxt, &alarm_ctxt).await {
                                error!("Failed to handle Open Pipe message: {}",e);
                                return;
                            }
                    }
                }
            }
        }
    }
    info!("Server exiting");
}
