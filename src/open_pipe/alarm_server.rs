use super::connection::{
    ErrorInfo, Message, MessageVariant, NotifyAlarm, NotifyAlarms, ParamWrapperCap,
    SubscribeAlarmParams,
};
use log::warn;
use std::collections::HashMap;
use std::sync::{Arc, Weak, Mutex};
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
    alarms: Vec<NotifyAlarm>,
}

impl AlarmServer {
    pub fn new() -> AlarmServer {
        AlarmServer {
            subscriptions: HashMap::new(),
            alarms: Vec::new(),
        }
    }

    fn build_notify_alarms(
        alarms: &[NotifyAlarm],
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
            alarm_notifications.push(alarm.clone());
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

    pub fn handle_message(&mut self, msg: Message, notify_fn: &Weak<ReplyFn>) -> Option<Message> {
        match msg.message {
            MessageVariant::SubscribeAlarm(ParamWrapperCap { params }) => {
                Some(self.subscribe(params, &msg.client_cookie, notify_fn.clone()))
            }
            MessageVariant::UnsubscribeAlarm => Some(self.unsubscribe(&msg.client_cookie)),
            _ => None,
        }
    }
}
