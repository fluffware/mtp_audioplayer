use tokio::net::{UnixStream, unix::OwnedWriteHalf, UnixListener};
use tokio::io::AsyncWriteExt;
use tokio::io::{AsyncRead};
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::sync::mpsc::{self, Receiver, Sender};

use std::future::Future;
use std::fs::{remove_file, create_dir_all};

use std::path::Path;
use serde::{Serialize,Deserialize};
use serde_json;
use std::process;
use log::{debug,error,warn};
use tokio::pin;

pub type Result<T> = 
    std::result::Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;

pub struct Connection
{
    stream: OwnedWriteHalf,
    cookie_prefix: String,
    cookie_count: u32,
    replies: Receiver<Message>
}


#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all="PascalCase")]
pub struct ErrorInfo
{
    pub error_code: u32,
    pub error_description: String
}

impl std::error::Error for ErrorInfo {}

impl std::fmt::Display for ErrorInfo
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
    {
        write!(f, "{} (0x{:08x})", self.error_description, self.error_code)
    }
}

impl Default for ErrorInfo
{
    fn default() -> ErrorInfo
    {
	ErrorInfo{error_code: 0,
		  error_description: String::new()
	}
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all="PascalCase")]
pub struct SubscribeTagParams
{
    pub tags: Vec<String>
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all="PascalCase")]
pub struct ReadTagParams
{
    pub tags: Vec<String>
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all="PascalCase")]
pub struct TagData
{
    pub name: String,
    pub value: String,
    pub quality: String,
    pub quality_code: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all="PascalCase")]
pub struct NotifyTag
{
    #[serde(flatten)]
    pub data: TagData,
    
    pub time_stamp: String,
   
    #[serde(flatten)]
    pub error: ErrorInfo
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all="PascalCase")]
pub struct WriteTagValue
{
    pub name: String,
    pub value: String
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all="PascalCase")]
pub struct WriteTagParams
{
    pub tags: Vec<WriteTagValue>
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all="PascalCase")]
pub struct NotifyTags
{
    pub tags: Vec<NotifyTag>,
}


#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all="PascalCase")]
pub struct NotifyWriteTag
{
    pub name: String,
    #[serde(flatten)]
    pub error: ErrorInfo
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all="PascalCase")]
pub struct NotifyWriteTags
{
    pub tags: Vec<NotifyWriteTag>
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all="PascalCase")]
pub struct SubscribeAlarmParams
{
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_names: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_id: Option<u32>,
}


#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all="PascalCase")]
pub struct NotifyAlarm
{
    pub name: String,
    #[serde(rename="ID")]
    pub id: String,
    pub alarm_class_name: String,
    pub alarm_class_symbol: String,
    pub event_text: String,
    #[serde(rename="InstanceID")]
    pub instance_id: String,
    pub priority: String,
    pub state: String,
    pub state_text: String,
    pub state_machine: String,
    pub modification_time: String
}

// Serialize as 'Param: {...}'
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all="PascalCase")]
pub struct ParamWrapperCap<T> {
    pub params: T
}

impl<T> From<T> for ParamWrapperCap<T>
{
    fn from(v: T) -> Self
    {
	ParamWrapperCap{params: v}
    }
}

// Serialize as 'param: {...}
#[derive(Serialize, Deserialize, Debug)]
pub struct ParamWrapperLow<T> {
    pub params: T
}





#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all="PascalCase")]
pub struct NotifyAlarms
{
    pub alarms: Vec<NotifyAlarm>,
    
}


#[derive(Serialize, Deserialize, Debug)]
#[serde(tag="Message")]
pub enum MessageVariant
{
    // Tags
    SubscribeTag(ParamWrapperCap<SubscribeTagParams>),
    NotifySubscribeTag(ParamWrapperCap<NotifyTags>),
    ErrorSubscribeTag(ErrorInfo),
    UnsubscribeTag,
    NotifyUnsubscribeTag,
    ErrorUnsubscribeTag(ErrorInfo),
    ReadTag(ParamWrapperCap<ReadTagParams>),
    NotifyReadTag(ParamWrapperCap<NotifyTags>),
    ErrorReadTag(ErrorInfo),
    WriteTag(ParamWrapperCap<WriteTagParams>),
    NotifyWriteTag(ParamWrapperCap<NotifyWriteTags>),
    ErrorWriteTag(ErrorInfo),

    // Alarms
    SubscribeAlarm(ParamWrapperCap<SubscribeAlarmParams>),
    ErrorSubscribeAlarm(ErrorInfo),
    NotifySubscribeAlarm(ParamWrapperCap<NotifyAlarms>),
    UnsubscribeAlarm,
    NotifyUnsubscribeAlarm,
    ErrorUnsubscribeAlarm(ErrorInfo),
    ReadAlarm(ParamWrapperCap<SubscribeAlarmParams>),
    NotifyReadAlarm(ParamWrapperLow<NotifyAlarms>),
    ErrorReadAlarm(ErrorInfo),
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all="PascalCase")]
pub struct Message
{
    #[serde(flatten)]
    pub message: MessageVariant,
    pub client_cookie: String,
}

async fn read_connection<R>(r: R, send: Sender<Message>)
    where R: AsyncRead + Unpin
{
    let mut r = BufReader::new(r);
    loop {
        let mut line = String::new();
        match r.read_line(&mut line).await {
            Err(e) => {
                error!("Failed to read line from pipe: {}", e);
                break;
            },
            Ok(l) => {
                if l == 0 {
                    break;
                }
                debug!("Got line: {}",line);
                match serde_json::from_str(&line) {
                    Err(e) => {
                        error!("Failed to parse message: {}", e);
                    },
                    Ok(msg) => {
                        send.send(msg).await.unwrap();
                    }
                }
                
                
            }
        }
        
    }

}

async fn send_cmd(stream: &mut OwnedWriteHalf, cmd: &Message) ->Result<()>
{
    let mut cmd_bytes = serde_json::to_vec(cmd)?;
    cmd_bytes.push(b'\n');
    debug!("Cmd: {}",String::from_utf8(cmd_bytes.clone()).unwrap());
    stream.write_all(&cmd_bytes).await?;
    stream.flush().await?;
    Ok(())
}

impl Connection {
    pub async fn connect<P>(path: P) -> std::io::Result<Connection>
    where P: AsRef<Path>
    {
        let stream = UnixStream::connect(path).await?;
        Ok(Self::from_stream(stream))
    }

    fn from_stream(stream: UnixStream) -> Connection
    {
	let (r,w) = stream.into_split();
        let (msg_in, msg_out) = mpsc::channel(10);
        tokio::spawn(read_connection(r, msg_in));
        Connection {
	    stream: w,
	    cookie_prefix: format!("cookie_{}_", process::id()),
	    cookie_count: 0,
	    replies: msg_out
        }
    }
    
    fn get_cookie(&mut self) -> String
    {
        self.cookie_count = self.cookie_count.wrapping_add(1);
        self.cookie_prefix.clone()+&self.cookie_count.to_string()
    }

    pub async fn get_message(&mut self) -> Option<Message> {
        self.replies.recv().await
    }

    pub async fn send_message(&mut self, msg: &Message) -> Result<()>
    {
	send_cmd(&mut self.stream, msg).await?;
	Ok(())
    }
    
    pub async fn subscribe_tags(&mut self, tags: &[&str])
                                -> Result<String> 
    {
        let cmd = Message {
            message: MessageVariant::SubscribeTag(ParamWrapperCap{
                params: SubscribeTagParams {
                    tags: tags.iter().map(|t| String::from(*t)).collect(),
                }}),
            client_cookie: self.get_cookie()
        };
        send_cmd(&mut self.stream, &cmd).await?;
        Ok(cmd.client_cookie)
    }

    pub async fn notify_subscibe_tags(&mut self,
				      tags: NotifyTags,
				      cookie: &str)
				      -> Result<()> 
    {
	let cmd = Message {
            message: MessageVariant::NotifySubscribeTag(tags.into()),
            client_cookie: cookie.to_string()
        };
	send_cmd(&mut self.stream, &cmd).await?;
        Ok(())
    }
    
    pub async fn error_subscibe_tags(&mut self,
				     error_code: u32,
				     error_description: String,
				      cookie: &str)
				      -> Result<()> 
    {
	let cmd = Message {
            message: MessageVariant::ErrorSubscribeTag(
		ErrorInfo{error_code, error_description}),
            client_cookie: cookie.to_string()
        };
	send_cmd(&mut self.stream, &cmd).await?;
        Ok(())
    }
    
    pub async fn unsubscribe_tags(&mut self, cookie: &str)
                                  -> Result<String> 
    {
         let cmd = Message {
            message: MessageVariant::UnsubscribeTag,
            client_cookie: cookie.to_string()
         };
        send_cmd(&mut self.stream, &cmd).await?;
        Ok(cmd.client_cookie)
    }
    
    pub async fn notify_unsubscibe_tags(&mut self,
					cookie: &str)
					-> Result<()> 
    {
	let cmd = Message {
            message: MessageVariant::NotifyUnsubscribeTag,
            client_cookie: cookie.to_string()
        };
	send_cmd(&mut self.stream, &cmd).await?;
        Ok(())
    }
    
    pub async fn write_tags(&mut self, tags: &[WriteTagValue]) -> Result<()>
    {
        let cmd = Message {
            message: MessageVariant::WriteTag(ParamWrapperCap{
                params: WriteTagParams {
                    tags: tags.to_vec(),
                }}),
            client_cookie: self.get_cookie()
        };
        send_cmd(&mut self.stream, &cmd).await?;
        Ok(())
    }
    
    pub async fn subscribe_alarms(&mut self) -> Result<String> 
    {
        let cmd = Message {
            message: MessageVariant::SubscribeAlarm(ParamWrapperCap{
                params: SubscribeAlarmParams {
                    system_names: None,
                    filter: None,
                    language_id: None
                }}),
            client_cookie: self.get_cookie()
        };
        send_cmd(&mut self.stream, &cmd).await?;
        Ok(cmd.client_cookie)
    }

    pub async fn unsubscribe_alarms(&mut self, cookie: &str)
                                  -> Result<String> 
    {
         let cmd = Message {
            message: MessageVariant::UnsubscribeAlarm,
            client_cookie: cookie.to_string()
        };
        send_cmd(&mut self.stream, &cmd).await?;
        Ok(cmd.client_cookie)
    }

    
}

 pub async fn listen<P, H, F, S>(path: P, handler: H, shutdown: S) 
			-> std::io::Result<()>
where H: Fn(Connection) -> F,
      F: Future<Output = ()> + Send + 'static,
      P: AsRef<Path>,
      S: Future<Output = ()> + Send + 'static,
{
     if let Some(parent) = path.as_ref().parent() {
	 create_dir_all(parent)?;
     }
     let listener = UnixListener::bind(path.as_ref())?;
     pin!(shutdown);
     loop {
	 tokio::select! {
             res = listener.accept() => {
		 let (stream, _addr) = res?;
		 let conn = Connection::from_stream(stream);
		 tokio::spawn(handler(conn));
	     },
	     _ = (&mut shutdown) => break
	 }
     }

     if let Err(e) = remove_file(path.as_ref()) {
	 warn!("Failed to delete named pipe {}: {}", path.as_ref().to_string_lossy(), e);
     }
     Ok(())
 }

#[test]
    fn serialize_test()
{
    let reply = Message{
        message: MessageVariant::NotifySubscribeTag(ParamWrapperCap {
            params: NotifyTags {
                tags: vec![
                    NotifyTag {
			data: TagData {
                            name: "Value".to_string(),
                            quality: "Good".to_string(),
                            quality_code: 192,
                            value: "32".to_string(),
			},
                        time_stamp: "2021-03-23T11:23:11Z".to_string(),
                        error: ErrorInfo {
                            error_code: 29100,
                            error_description: "Error".to_string()
                        }
                        
                    }   
                ]
            }}),
        client_cookie: "hksjdhljdfhjhioehuh".to_string()
    };
    println!("{}",serde_json::ser::to_string(&reply).unwrap());
}
