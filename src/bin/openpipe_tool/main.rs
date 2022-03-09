use clap::{Arg, Command};
use futures::stream::StreamExt;
use futures::FutureExt;
use futures::SinkExt;
use log::{debug, error, info};
//use mtp_audioplayer::open_pipe::alarm_data::AlarmData;
use mtp_audioplayer::open_pipe::{
    alarm_server::AlarmServer,
    connection::{self, Connection, MessageVariant},
    tag_server::{ReplyFn, TagServer},
};
use serde_json;
use std::sync::{Arc, Mutex, Weak};
use tokio::signal;
use tokio::sync::mpsc::UnboundedSender;
//use tokio::time::{timeout, Duration};
use mtp_audioplayer::util::error::DynResult;
use std::env;
use std::net::IpAddr;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;
use warp::ws::Message as WsMessage;
use warp::{Filter, Reply};

async fn open_pipe_handler(
    mut conn: Connection,
    tag_server: Arc<Mutex<TagServer>>,
    alarm_server: Arc<Mutex<AlarmServer>>,
) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let notify_fn: Arc<ReplyFn> = Arc::new(Mutex::new(move |msg| {
        if let Err(err) = tx.send(msg) {
            error!("Failed to queue reply: {}", err);
        }
        Ok(()) as DynResult<()>
    }));
    let notify_fn_weak = Arc::downgrade(&notify_fn);
    loop {
        tokio::select! {
            res = conn.get_message() => {
                match res {
                    Ok(msg) => {
                        let reply = match msg.message {
                            MessageVariant::SubscribeTag(_) |
                            MessageVariant::UnsubscribeTag |
                            MessageVariant::ReadTag(_) |
                            MessageVariant::WriteTag(_) => {
                                let mut tag_server = tag_server.lock().unwrap();
                                tag_server.handle_message(msg, &notify_fn_weak)
                            },
                            MessageVariant::SubscribeAlarm(_) |
                            MessageVariant::UnsubscribeAlarm |
                            MessageVariant::NotifySubscribeAlarm(_) |
                            MessageVariant::ReadAlarm(_) => {
                                let mut alarm_server = alarm_server.lock().unwrap();
                                alarm_server.handle_message(msg, &notify_fn_weak)
                            },

                            _ => None
                        };

                        if let Some(reply) = reply {
                            debug!("Reply: {:?}", &reply);
                            if let Err(err) = conn.send_message(&reply).await {
                                error!("Failed to send Open Pipe message: {}", err);

                            }
                        }
                    },
                    Err(e) => {
                        error!("Failed to get message: {}",e);
                        break
                    }
                }
            },
            res = rx.recv() => {
                match res {
                    Some(notice) => {
                        if let Err(err) = conn.send_message(&notice).await {
                            error!("Failed to receive Open Pipe message: {}", err);
                        }
                    },
                    None => break
                }
            }
        }
    }
    debug!("Connection closed");
}

fn web_handler(
    ws_msg: WsMessage,
    tag_server: &Arc<Mutex<TagServer>>,
    alarm_server: &Arc<Mutex<AlarmServer>>,
    notify: &Weak<ReplyFn>,
    tx: &mut UnboundedSender<connection::Message>,
) {
    if let Ok(json) = ws_msg.to_str() {
        match serde_json::from_str::<connection::Message>(json) {
            Ok(op_msg) => match op_msg.message {
                MessageVariant::SubscribeTag(_)
                | MessageVariant::NotifySubscribeTag(_)
                | MessageVariant::ErrorSubscribeTag(_)
                | MessageVariant::UnsubscribeTag
                | MessageVariant::NotifyUnsubscribeTag
                | MessageVariant::ErrorUnsubscribeTag(_)
                | MessageVariant::ReadTag(_)
                | MessageVariant::NotifyReadTag(_)
                | MessageVariant::ErrorReadTag(_)
                | MessageVariant::WriteTag(_)
                | MessageVariant::NotifyWriteTag(_)
                | MessageVariant::ErrorWriteTag(_) => {
                    let mut tag_server = tag_server.lock().unwrap();

                    if let Some(msg) = tag_server.handle_message(op_msg, &notify) {
                        if let Err(err) = tx.send(msg) {
                            error!("Failed to queue reply: {}", err);
                        }
                    }
                }
                MessageVariant::SubscribeAlarm(_)
                | MessageVariant::NotifySubscribeAlarm(_)
                | MessageVariant::ErrorSubscribeAlarm(_)
                | MessageVariant::UnsubscribeAlarm
                | MessageVariant::NotifyUnsubscribeAlarm
                | MessageVariant::ErrorUnsubscribeAlarm(_)
                | MessageVariant::ReadAlarm(_)
                | MessageVariant::NotifyReadAlarm(_)
                | MessageVariant::ErrorReadAlarm(_) => {
                    let mut alarm_server = alarm_server.lock().unwrap();
                    if let Some(msg) = alarm_server.handle_message(op_msg, &notify) {
                        if let Err(err) = tx.send(msg) {
                            error!("Failed to queue reply: {}", err);
                        }
                    }
                }
            },
            Err(err) => error!("Failed to parse request from web page: {}", err),
        }
    }
}

/*
async fn subscribe_tags(
    pipe: &mut Connection,
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
}*/

/*
async fn subscribe_alarms(pipe: &mut Connection) -> DynResult<Vec<AlarmData>> {
    debug!("Subcribing alarms");
    let _subscription = pipe.subscribe_alarms().await?;
    let alarms;
    'next_event: loop {
        match timeout(Duration::from_secs(5), pipe.get_message()).await {
            Err(_) => {
                return Err("No reply for alarm subscription".to_string().into());
            }
            Ok(res) => match res {
                Ok(event) => {
                    debug!("Get subscribe reply: {:?}", event);
                    match event.message {
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
                    }
                }
                Err(e) => {
                    return Err(e.into());
                }
            },
        }
    }
    Ok(alarms)
}
*/

fn setup_client(open_pipe_path: &str) -> Arc<dyn Fn(warp::ws::Ws) -> Box<dyn Reply> + Send + Sync> {
    let open_pipe_path = Arc::new(open_pipe_path.to_owned());
    Arc::new(move |ws: warp::ws::Ws| {
        let open_pipe_path = open_pipe_path.clone();
        Box::new(ws.on_upgrade(|websocket| async move {
                let mut open_pipe_conn = match Connection::connect(&open_pipe_path).await {
                    Ok(c) => c,
                    Err(e) => {
                        error!("{}", e);
                        return;
                    }
                };
                let (mut tx, mut rx) = websocket.split();

                loop {
                    tokio::select! {
                    res = open_pipe_conn.get_message() => {
                        match res {
                            Ok(op_msg) => {
                                match serde_json::to_string(&op_msg) {
                                    Ok(json) =>
                                    {
                                        if let Err(err) = tx.send(WsMessage::text(json)).await {
                                            error!("Failed to send message to web: {}", err);
                                        }
                                    },
                                    Err(e) => error!("Invalid JSON: {}", e)
                                }
                            },
                            Err(e) => error!("Failed to receive from pipe {}", e)
                        }
                    },
                    res = rx.next() => {
                        match res {
                            Some(Ok(ws_msg)) => match ws_msg.to_str() {
                                Ok(json) => match serde_json::from_str::<connection::Message>(json) {
                                    Ok(op_msg) => {
                                        if let Err(err) = open_pipe_conn.send_message(&op_msg).await {
                                            error!("Failed to send message to pipe: {}", err);
                                        }
                                    },
                                    Err(e) => error!("Invalid json: {}", e)
                                },
                                Err(_) => error!("Websocket message is not a string"),
                            },
                            Some(Err(e)) => error!("Failed to receive from web: {}", e),

                            None => break
                        }
                    }
                }
            }}))
    })
}

fn setup_server(
    tag_server: &Arc<Mutex<TagServer>>,
    alarm_server: &Arc<Mutex<AlarmServer>>,
) -> Arc<dyn Fn(warp::ws::Ws) -> Box<dyn Reply> + Send + Sync> {
    let tag_server_web = tag_server.clone();
    let alarm_server_web = alarm_server.clone();
    Arc::new(move |ws: warp::ws::Ws| {
        // And then our closure will be called when it completes...
        let tag_server = tag_server_web.clone();
        let alarm_server = alarm_server_web.clone();
        Box::new(ws.on_upgrade(|websocket| async move {
                let (mut tx, rx) = websocket.split();
                let (send_tx, mut recv_tx) =
                    tokio::sync::mpsc::unbounded_channel::<connection::Message>();
                let send_tx_web = send_tx.clone();
                let notify: Arc<ReplyFn> = Arc::new(Mutex::new(move |msg| {
                    debug!("Socket sent: {:?}", msg);
                    if let Err(err) = send_tx_web.send(msg) {
                        error!("Failed to queue reply: {}", err);
                    }
                    Ok(())
                }));
                let notify_weak = Arc::downgrade(&notify);
                let tag_server = tag_server.clone();
                tokio::select! {
                    _ = async move {
                        while let Some(msg) = recv_tx.recv().await {
                            match serde_json::to_string(&msg) {
                                Ok(json) => {
                                    if let Err(err) = tx.send(WsMessage::text(json)).await {
                                        error!("Failed to send web message: {}", err);
                                    }
                                },
                                Err(e) => error!("Failed to create JSON reply: {}", e)
                            }
                        }
                    } => {},
                    _ = rx.for_each(move |res| {
                        let tag_server = tag_server.clone();
                        let notify_weak = notify_weak.clone();
                        let mut send_tx = send_tx.clone();
                        let tag_server = tag_server.clone();
                        let alarm_server = alarm_server.clone();
                        async move {
                            println!("Msg: {:?}",res);
                            if let Ok(msg) = res {
                                web_handler(msg, &tag_server, &alarm_server, &notify_weak, &mut send_tx);
                            }
                        }
                    }) => {}
                }
            }))
    })
}

#[cfg(target_os = "linux")]
const DEFAULT_PIPE_NAME: &str = "/tmp/siemens/automation/HmiRunTime";
#[cfg(windows)]
const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\HmiRuntime";

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let app_args = Command::new("Open Pipe tool")
        .version("0.1")
        .about("Test tool for Open Pipe protocol")
        .arg(Arg::new("client").long("client"))
        .arg(
            Arg::new("http-port")
                .long("http-port")
                .takes_value(true)
                .default_value("9229"),
        )
        .arg(
            Arg::new("http-bind")
                .long("http-bind")
                .takes_value(true)
                .default_value("127.0.0.1"),
        )
        .arg(Arg::new("file-root").long("file-root").takes_value(true))
        .arg(
            Arg::new("pipe")
                .long("pipe")
                .takes_value(true)
                .default_value(DEFAULT_PIPE_NAME),
        );

    let args = app_args.get_matches();

    let http_port = match args.value_of("http-port") {
        Some(s) => match s.parse::<u16>() {
            Ok(port) => port,
            Err(_) => {
                error!("Invalid value for HTTP port");
                return;
            }
        },
        None => {
            error!("No value for HTTP port");
            return;
        }
    };

    let shutdown = CancellationToken::new();
    let open_pipe_path = args.value_of("pipe").unwrap().to_owned();
    let mut open_pipe_connection;
    let ws_run;
    if args.is_present("client") {
        ws_run = setup_client(&open_pipe_path);
        let shutdown = shutdown.clone();
        open_pipe_connection = tokio::spawn(async move {
            shutdown.cancelled().await;
            Ok(())
        })
        .fuse()
    } else {
        let tag_server = Arc::new(Mutex::new(TagServer::new(true)));
        let alarm_server = Arc::new(Mutex::new(AlarmServer::new()));
        ws_run = setup_server(&tag_server, &alarm_server);
        let shutdown_open_pipe = {
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        };

        open_pipe_connection = tokio::spawn(async move {
            connection::listen(
                &open_pipe_path,
                move |conn| open_pipe_handler(conn, tag_server.clone(), alarm_server.clone()),
                shutdown_open_pipe,
            )
            .await
        })
        .fuse();
    }

    let mut file_root: PathBuf;
    if let Some(path) = args.value_of("file-root") {
        file_root = PathBuf::from(path);
    } else if let Ok(path) = env::current_exe() {
        file_root = path.parent().unwrap().join("../share/openpipe_tool");
    } else {
        file_root = PathBuf::from("web");
    }
    if !file_root.is_dir() {
        file_root = PathBuf::from(".");
    }
    let index_file = file_root.join("index.html");
    if !index_file.exists() {
        error!("{} does not exist", index_file.to_string_lossy());
        return;
    }

    let ws_filter = warp::path("open_pipe")
        .and(warp::ws())
        .map(move |ws: warp::ws::Ws| ws_run(ws));
    let files = warp::path("files").and(warp::fs::dir(file_root));
    let root = ws_filter.or(files);
    let web_server = warp::serve(root);
    let shutdown_web = shutdown.clone();
    let http_bind = match args.value_of("http-bind") {
        Some(s) => match s.parse::<IpAddr>() {
            Ok(addr) => addr,
            Err(_) => {
                error!("Invalid local address HTTP server");
                return;
            }
        },
        None => {
            error!("No value for HTTP port");
            return;
        }
    };
    let mut web_server = tokio::spawn(
        web_server
            .bind_with_graceful_shutdown((http_bind, http_port), async move {
                shutdown_web.cancelled().await
            })
            .1,
    )
    .fuse();

    let mut open_pipe_server_running = true;
    let mut web_server_running = true;
    while web_server_running || open_pipe_server_running {
        tokio::select! {
            res = signal::ctrl_c() => {
            shutdown.cancel();
                if let Err(e) = res {
                    error!("Failed to wait for ctrl-c: {}",e);
                }
            },
            h = (&mut web_server) => {
                shutdown.cancel();
                if let Err(e) = h {
                    error!("Web server failed: {}",e)
                }
            web_server_running = false;
            },
            h = (&mut open_pipe_connection) => {
                shutdown.cancel();
                if let Ok(Err(e)) = h {
                    error!("Open Pipe server failed: {}",e)
                }
        open_pipe_server_running = false;
            }
        }
    }

    info!("Server exiting");
}
