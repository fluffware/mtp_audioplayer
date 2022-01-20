use super::alarm_data::AlarmData;
use super::connection::{
    ErrorInfo, Message, MessageVariant, NotifyAlarm, NotifyAlarms, ParamWrapperCap,
    SubscribeAlarmParams,
};
use log::{error, warn, debug};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;

pub type ReplyFn = Mutex<dyn FnMut(Message) -> Result<()> + Send>;

struct Subscription {
    system_names: Option<Vec<String>>,
    filter: Option<String>,
    language_id: Option<u32>,
    notify: Weak<ReplyFn>,
    cookie: String,
}

pub struct AlarmServer {
    // Maps client cookies to subscriptions
    subscriptions: HashMap<String, Arc<Mutex<Subscription>>>,
    alarms: Vec<AlarmData>,
}

impl AlarmServer {
    pub fn new() -> AlarmServer {
        AlarmServer {
            subscriptions: HashMap::new(),
            alarms: Vec::new(),
        }
    }

    fn build_notify_alarms(
        alarms: &[AlarmData],
        system_names: &Option<&[String]>,
        filter: &Option<&str>,
    ) -> NotifyAlarms {
        let mut alarm_notifications: Vec<NotifyAlarm> = Vec::new();
        if system_names.is_some() {
            warn!("Can't filter on system names");
        }

        if filter.is_some() {
            warn!("Alarm filters not implemented");
        }
        for alarm in alarms {
            // TODO Implement filtering
            alarm_notifications.push(NotifyAlarm::from(alarm));
        }
        NotifyAlarms {
            alarms: alarm_notifications,
        }
    }

    fn subscribe(
        &mut self,
        params: SubscribeAlarmParams,
        cookie: &str,
        notify: Weak<ReplyFn>,
    ) -> Message {
        let SubscribeAlarmParams {
            system_names,
            filter,
            language_id,
        } = params;
        let subscr = Subscription {
            system_names,
            filter,
            language_id,
            notify,
            cookie: cookie.to_string(),
        };
        let msg = Message {
            message: MessageVariant::NotifySubscribeAlarm(
                Self::build_notify_alarms(
                    &self.alarms,
                    &subscr.system_names.as_deref(),
                    &subscr.filter.as_deref(),
                )
                .into(),
            ),
            client_cookie: subscr.cookie.clone(),
        };
	self.subscriptions.insert(cookie.to_string(), Arc::new(Mutex::new(subscr)));
	debug!("Added subscriber {}", cookie);
        msg
    }

    fn unsubscribe(&mut self, cookie: &str) -> Message {
        if self.subscriptions.remove(cookie).is_some() {
            Message {
                message: MessageVariant::NotifyUnsubscribeAlarm,
                client_cookie: cookie.to_string(),
            }
        } else {
            Message {
                message: MessageVariant::ErrorUnsubscribeAlarm(ErrorInfo {
                    error_code: 4,
                    error_description: "No matching subscription".to_string(),
                }),
                client_cookie: cookie.to_string(),
            }
        }
    }

    fn notify_subscribe(&mut self, params: NotifyAlarms, cookie: &str) {
	debug!("notify_subscribe: {}", cookie);
        let alarms: Vec<AlarmData> = params
            .alarms
            .into_iter()
            .map(|alarm| AlarmData::from(alarm))
            .collect();
        for (subscr_cookie, subscr) in &self.subscriptions {
	    debug!("subscr: {}", subscr_cookie);
            let subscr = subscr.lock().unwrap();
            if subscr_cookie != cookie {
                let notify = Self::build_notify_alarms(
                    &alarms,
                    &subscr.system_names.as_deref(),
                    &subscr.filter.as_deref(),
                );
                if let Some(reply) = Weak::upgrade(&subscr.notify) {
                    println!("Notified alarm: {} from {}", subscr_cookie, cookie);
                    if let Err(e) = reply.lock().unwrap()(Message {
                        message: MessageVariant::NotifySubscribeAlarm(ParamWrapperCap {
                            params: notify,
                        }),
                        client_cookie: cookie.to_string(),
                    }) {
                        error!("Failed to send alarm notify: {}", e);
                    }
                }
            }
        }
        for alarm in alarms {
            match self.alarms.binary_search_by(|a| a.cmp_id(&alarm)) {
                Ok(p) => {
                    self.alarms[p].state = alarm.state;
                    self.alarms[p].modification_time = alarm.modification_time;
                }
                Err(p) => {
                    self.alarms.insert(p, alarm);
                }
            }
        }
    }

    pub fn handle_message(&mut self, msg: Message, notify_fn: &Weak<ReplyFn>) -> Option<Message> {
        match msg.message {
            MessageVariant::SubscribeAlarm(ParamWrapperCap { params }) => {
                Some(self.subscribe(params, &msg.client_cookie, notify_fn.clone()))
            }
            MessageVariant::UnsubscribeAlarm => Some(self.unsubscribe(&msg.client_cookie)),
            MessageVariant::NotifySubscribeAlarm(ParamWrapperCap { params }) => {
                self.notify_subscribe(params, &msg.client_cookie);
                None
            }
            _ => None,
        }
    }
}
