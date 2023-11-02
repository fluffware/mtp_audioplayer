use log::debug;
use std::collections::HashMap;
use std::sync::{Mutex, Weak};
//use log::{debug};
use super::connection::{
    ErrorInfo, Message, MessageVariant, NotifyTag, NotifyTags, NotifyWriteTag, NotifyWriteTags,
    ParamWrapperCap, SubscribeTagParams, TagData, WriteTagParams, WriteTagValue,
};
use chrono::offset::Utc;
use std::collections::HashSet;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;

pub type ReplyFn = Mutex<dyn FnMut(Message) -> Result<()> + Send>;

struct Subscription {
    tags: Vec<String>, // Empty means any tag
    notify: Weak<ReplyFn>,
    cookie: String,
}

pub struct TagServer {
    // Maps client cookies to subscriptions
    subscriptions: HashMap<String, Subscription>,
    // All tags
    tags: HashMap<String, TagData>,
    populate: bool, // Implicitly add any subscribed tag
}

impl TagServer {
    pub fn new(populate: bool) -> TagServer {
        TagServer {
            subscriptions: HashMap::new(),
            tags: HashMap::new(),
            populate,
        }
    }

    fn build_notify_tags(tag_map: &HashMap<String, TagData>, tags: &[String]) -> NotifyTags {
        let mut tag_notifications = Vec::new();
        if tags.is_empty() {
            for tag_data in tag_map.values() {
                tag_notifications.push(NotifyTag {
                    data: (*tag_data).clone(),
                    time_stamp: Utc::now().to_rfc3339(),
                    error: ErrorInfo::default(),
                });
            }
        } else {
            for tag in tags {
                if let Some(tag_data) = tag_map.get(tag) {
                    tag_notifications.push(NotifyTag {
                        data: (*tag_data).clone(),
                        time_stamp: Utc::now().to_rfc3339(),
                        error: ErrorInfo::default(),
                    });
                }
            }
        }
        NotifyTags {
            tags: tag_notifications,
        }
    }

    fn subscribe(&mut self, tags: &[String], cookie: &str, notify: Weak<ReplyFn>) -> Message {
        debug!("subscribe: {:?}", tags);
        if self.populate {
            for tag in tags {
                if !self.tags.contains_key(tag) {
                    self.tags.insert(
                        tag.to_string(),
                        TagData {
                            name: tag.to_string(),
                            value: "0".to_string(),
                            quality: "Good".to_string(),
                            quality_code: 192,
                        },
                    );
                }
            }
        }
        let subscr = Subscription {
            tags: Vec::from(tags),
            notify,
            cookie: cookie.to_string(),
        };

        let tags = Self::build_notify_tags(&self.tags, &subscr.tags);
        let msg = Message {
            message: MessageVariant::NotifySubscribeTag(tags.into()),
            client_cookie: subscr.cookie.clone(),
        };
        self.subscriptions.insert(cookie.into(), subscr);

        msg
    }

    fn unsubscribe(&mut self, cookie: &str) -> Message {
        if self.subscriptions.remove(cookie).is_some() {
            Message {
                message: MessageVariant::NotifyUnsubscribeTag,
                client_cookie: cookie.to_string(),
            }
        } else {
            Message {
                message: MessageVariant::ErrorUnsubscribeTag(ErrorInfo {
                    error_code: 4,
                    error_description: "No matching subscription".to_string(),
                }),
                client_cookie: cookie.to_string(),
            }
        }
    }

    pub fn set_tag_value(&mut self, tag: &str, value: &str, notifications: &mut HashSet<String>) {
        debug!("Setting tag {} = {}", tag, value);
        match self.tags.get_mut(tag) {
            None => {
                let tag_data = TagData {
                    name: tag.into(),
                    quality: "Good".to_string(),
                    quality_code: 192,
                    value: value.to_string(),
                };
                self.tags.insert(tag.to_string(), tag_data);
            }
            Some(tag_data) => {
                tag_data.value = value.to_string();
            }
        };
        notifications.insert(tag.to_string());
    }

    fn send_tag_notifications(
        &mut self,
        notifications: &HashSet<String>,
        exclude_cookie: Option<&str>,
    ) {
        let tag_map = &self.tags;
        self.subscriptions.retain(|subscr_name, subscr| {
            // Check if subscription is still active
            let notify_fn = match Weak::upgrade(&subscr.notify) {
                Some(notify_fn) => notify_fn,
                None => {
                    debug!("Dropped subscription");
                    return false; // Remove subscription
                }
            };

            let mut found;
            if match exclude_cookie {
                Some(cookie) => cookie == subscr.cookie,
                None => false,
            } {
                found = false;
            } else if subscr.tags.is_empty() {
                found = true;
            } else {
                found = false;
                for tag in &subscr.tags {
                    if notifications.contains(tag) {
                        found = true;
                        break;
                    }
                }
            }
            if found
                && match exclude_cookie {
                    Some(cookie) => cookie != subscr.cookie,
                    None => true,
                }
            {
                let msg = Message {
                    message: MessageVariant::NotifySubscribeTag(
                        Self::build_notify_tags(tag_map, &subscr.tags).into(),
                    ),
                    client_cookie: subscr.cookie.clone(),
                };
                let mut send = notify_fn.lock().unwrap();
                let _ = send(msg);
                debug!("Notifying subscription {}", subscr_name);
            }

            true // Keep subscription
        });
    }

    fn write_tags(&mut self, tag_values: &[WriteTagValue], cookie: &str) -> Message {
        let mut tag_result = Vec::new();
        let mut notifications = HashSet::new();
        for WriteTagValue { name, value } in tag_values {
            self.set_tag_value(name, value, &mut notifications);
            tag_result.push(NotifyWriteTag {
                name: name.clone(),
                error: ErrorInfo::default(),
            });
        }
        self.send_tag_notifications(&notifications, Some(cookie));
        Message {
            message: MessageVariant::NotifyWriteTag(ParamWrapperCap {
                params: NotifyWriteTags { tags: tag_result },
            }),
            client_cookie: cookie.to_string(),
        }
    }

    pub fn handle_message(&mut self, msg: Message, notify_fn: &Weak<ReplyFn>) -> Option<Message> {
        match msg.message {
            MessageVariant::SubscribeTag(ParamWrapperCap {
                params: SubscribeTagParams { tags },
            }) => Some(self.subscribe(&tags, &msg.client_cookie, notify_fn.clone())),
            MessageVariant::UnsubscribeTag => Some(self.unsubscribe(&msg.client_cookie)),
            MessageVariant::WriteTag(ParamWrapperCap {
                params: WriteTagParams { tags },
            }) => Some(self.write_tags(&tags, &msg.client_cookie)),

            _ => None,
        }
    }
}

#[cfg(test)]
use std::sync::Arc;

#[test]
fn test_subscribe() {
    let mut server = TagServer::new(false);
    let mut notifications = HashSet::new();
    server.set_tag_value("Tag0", "0", &mut notifications);
    server.set_tag_value("Tag1", "1", &mut notifications);
    let mut notify: Arc<ReplyFn> = Arc::new(Mutex::new(|msg| {
        println!("Notify: {:?}", msg);
        Ok(())
    }));

    server.subscribe(
        &["Tag0".to_string(), "Tag1".to_string()],
        "dsjalk",
        Arc::downgrade(&notify),
    );
    server.set_tag_value("Tag1", "2", &mut notifications);
    server.send_tag_notifications(&notifications, None);
    server.unsubscribe("dsjalk");
}
