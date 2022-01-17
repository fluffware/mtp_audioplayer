use futures::stream::StreamExt;
use futures::FutureExt;
use futures::SinkExt;
use log::{debug, error, info};
use mtp_audioplayer::open_pipe::{
    alarm_server::AlarmServer,
    connection::{self, Connection, MessageVariant},
    tag_server::{ReplyFn, TagServer},
};
use serde_json;
use std::sync::{Arc, Mutex, Weak};
use tokio::pin;
use tokio::signal;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use warp::ws::Message as WsMessage;
use warp::Filter;

pub type DynResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;

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
        Ok(())
    }));
    let notify_fn_weak = Arc::downgrade(&notify_fn);
    loop {
        tokio::select! {
            res = conn.get_message() => {
                match res {
                    Some(msg) => {
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
                    None => break
                }
            },
            res = rx.recv() => {
                match res {
                    Some(notice) => {
                        if let Err(err) = conn.send_message(&notice).await {
                            error!("Failed to send Open Pipe message: {}", err);
                        }
                    },
                    None => break
                }
            }
        }
    }
    debug!("Connection closed");
}

fn tag_web_handler(
    ws_msg: WsMessage,
    tag_server: &Arc<Mutex<TagServer>>,
    notify: &Weak<ReplyFn>,
    tx: &mut UnboundedSender<connection::Message>,
) {
    if let Ok(json) = ws_msg.to_str() {
        match serde_json::from_str(json) {
            Ok(op_msg) => {
                let mut tag_server = tag_server.lock().unwrap();
                if let Some(msg) = tag_server.handle_message(op_msg, &notify) {
                    if let Err(err) = tx.send(msg) {
                        error!("Failed to queue reply: {}", err);
                    }
                }
            }
            Err(err) => error!("Failed to parse request from web page: {}", err),
        }
    }
}

fn alarm_web_handler(
    ws_msg: WsMessage,
    alarm_server: &Arc<Mutex<AlarmServer>>,
    notify: &Weak<ReplyFn>,
    tx: &mut UnboundedSender<connection::Message>,
) {
    if let Ok(json) = ws_msg.to_str() {
        match serde_json::from_str(json) {
            Ok(op_msg) => {
                let mut alarm_server = alarm_server.lock().unwrap();
                if let Some(msg) = alarm_server.handle_message(op_msg, &notify) {
                    if let Err(err) = tx.send(msg) {
                        error!("Failed to queue reply: {}", err);
                    }
                }
            }
            Err(err) => error!("Failed to parse request from web page: {}", err),
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let shutdown = CancellationToken::new();
    let tag_server = Arc::new(Mutex::new(TagServer::new(true)));
    let alarm_server = Arc::new(Mutex::new(AlarmServer::new()));
    let tag_server_web = tag_server.clone();
    let alarm_server_web = alarm_server.clone();
    let tags = warp::path("tags")
        // The `ws()` filter will prepare the Websocket handshake.
        .and(warp::ws())
        .map(move |ws: warp::ws::Ws| {
            // And then our closure will be called when it completes...
            let tag_server = tag_server_web.clone();
            ws.on_upgrade(|websocket| async move {
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
                        async move {
                            println!("Msg: {:?}",res);
                            if let Ok(msg) = res {
                                tag_web_handler(msg, &tag_server, &notify_weak, &mut send_tx);
                            }
                        }
                    }) => {}
                }
            })
        });
    let alarms = warp::path("alarms")
        // The `ws()` filter will prepare the Websocket handshake.
        .and(warp::ws())
        .map(move |ws: warp::ws::Ws| {
            // And then our closure will be called when it completes...
            let alarm_server = alarm_server_web.clone();
            ws.on_upgrade(|websocket| async move {
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
                let alarm_server = alarm_server.clone();
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
                        let alarm_server = alarm_server.clone();
                        let notify_weak = notify_weak.clone();
                        let mut send_tx = send_tx.clone();
                        async move {
                            println!("Msg: {:?}",res);
                            if let Ok(msg) = res {
                                alarm_web_handler(msg, &alarm_server, &notify_weak, &mut send_tx);
                            }
                        }
                    }) => {}
                }
            })
        });
    let files = warp::path("files").and(warp::fs::dir("src/bin/openpipe_tool/web"));

    let root = tags.or(alarms).or(files);
    let web_server = warp::serve(root);
    let shutdown_web = shutdown.clone();
    let mut web_server = tokio::spawn(
        web_server
            .bind_with_graceful_shutdown(([127, 0, 0, 1], 9229), async move {
                shutdown_web.cancelled().await
            })
            .1,
    )
    .fuse();

    let shutdown_open_pipe = {
        let shutdown = shutdown.clone();
        async move { shutdown.cancelled().await }
    };
    let mut open_pipe_server = tokio::spawn(async move {
        connection::listen(
            "/tmp/siemens/automation/HmiRunTime",
            move |conn| open_pipe_handler(conn, tag_server.clone(), alarm_server.clone()),
            shutdown_open_pipe,
        )
        .await
    })
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
            h = (&mut open_pipe_server) => {
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
