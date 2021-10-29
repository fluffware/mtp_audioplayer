use std::time::Duration;
use std::collections::HashMap;
use std::io::Read;
use xml::ParserConfig;
use std::error::Error;
use xml::reader::Result as XmlResult;
use xml::reader::XmlEvent;
use xml::name::OwnedName;
use xml::attribute::OwnedAttribute;
use crate::xml_stack::{TopElement, XmlSiblingIter};
use std::num::NonZeroU32;
use std::convert::TryInto;

#[derive(Debug)]
pub enum ConfigError
{
    UnexpectedEvent(XmlEvent),
    UnexpectedAttribute(String),
    MissingAttribute(String),
    InvalidState{file: String, line: u32, column: u32}
}


use ConfigError::*;

impl std::error::Error for ConfigError {}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>)
           -> std::result::Result<(), std::fmt::Error>
    {
        match self {
            UnexpectedEvent(event) => {
                match event {
                    XmlEvent::StartElement{name, ..} =>
                        write!(f, "Unexpected start tag '{}'", name.local_name),
                    XmlEvent::EndElement{name} =>
                        write!(f, "Unexpected end tag '{}'", name.local_name),
                    XmlEvent::Characters(text) =>
                        write!(f, "Unexpected text '{}'", text),
                    ev =>
                        write!(f, "Unexpected XML event '{:?}'", ev)
                }
            },
            UnexpectedAttribute(name) =>
                write!(f, "Unexpected attribute '{}'", name),
            MissingAttribute(name) =>
                write!(f, "Missing attribute '{}'", name),
            InvalidState{file, line, column} =>
                write!(f, "Invalid state at {}:{}:{}", file, line, column),
        }
    }
}

macro_rules! invalid_state {
    () => {InvalidState{file: file!().to_string(), 
                        line: line!(), 
                        column: column!()}
    }
}

#[derive(Debug)]
enum AttrError {
    Missing{element: OwnedName, attribute: OwnedName},
    //Unexpected{element: OwnedName, attribute: OwnedName},
    WrongEventType,
}

impl std::error::Error for AttrError
{
}

impl std::fmt::Display for AttrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>)
           -> std::result::Result<(), std::fmt::Error>
    {
        match self {
            AttrError::Missing{element,attribute} =>
                write!(f, "Missing attribute '{}' on element '{}'",
                       attribute.local_name, element.local_name),
            /*
            AttrError::Unexpected{element,attribute} =>
                write!(f, "Unexpected attribute '{}' on element '{}'",
                       attribute.local_name, element.local_name),
             */
            AttrError::WrongEventType =>
                write!(f, "Can only read attributes from StartElement events"),
        }
    }
}


fn get_attrs<const N: usize>(event: &XmlEvent,
                             attr_names: &[&str;N],
                             namespace: &Option<String>)
                             -> Result<[String;N], AttrError> 
{
    let attrs;
    let elem_name;
    if let XmlEvent::StartElement{name, attributes, ..} = event {
        attrs = attributes;
        elem_name = name;
    } else {
        return Err(AttrError::WrongEventType);
    }

    let mut out = Vec::with_capacity(N);
    for local in attr_names {
        if let Some(attr) = attrs.iter().find(|a| {
            local == &a.name.local_name 
                && namespace == &a.name.namespace}) {
            out.push(attr.value.clone());
        } else {
            return Err(AttrError::Missing{
                element: elem_name.clone(),
                attribute: OwnedName{local_name: local.to_string(), 
                                     namespace: namespace.clone(),
                                     prefix: None}})
        }
    }
    Ok(out.try_into().unwrap())
}                   

fn get_attr(event: &XmlEvent,
            attr_name: &str,
            namespace: &Option<String>)
            -> Result<Option<String>, AttrError>
{
    let attrs;
    if let XmlEvent::StartElement{attributes, ..} = event {
        attrs = attributes;
    } else {
        return Err(AttrError::WrongEventType);
    }

    let out;
    if let Some(attr) = attrs.iter().find(|a| {
        attr_name == a.name.local_name 
            && namespace == &a.name.namespace}) {
        out = Some(attr.value.clone());
    } else {
        out = None;
    }
    Ok(out)
}                   

#[derive(Debug)]
enum ClipType
{
    File(String)
}

#[derive(Debug)]
enum TagTriggerType
{
    Toggle,
    //Equals{value: i32}
}

#[derive(Debug)]
pub struct TagTriggerConfig
{
    trigger: TagTriggerType,
    action: ActionType
}

#[derive(Debug)]
pub enum ActionType
{
    Sequence(Vec<ActionType>),
    Parallel(Vec<ActionType>),
    Play{sound: String},
    Wait(Duration),
    Reference(String),
    // No count means forever.
    Repeat{count: Option<NonZeroU32>, action: Box<ActionType>},
    AlarmRestart,
    SetProfile{profile: String}
}

#[derive(Debug)]
pub struct ActionConfig
{
    id: String,
    action: ActionType
}

#[derive(Debug)]
pub struct ProfileConfig
{
}

#[derive(Debug)]
pub struct PlayerConfig
{
    bind: String,
    playback_device: String,
    rate: u32,
    channels: u8,
    clip_root: String,
    clips: HashMap<String,ClipType>,
    tags: HashMap<String, TagTriggerConfig>,
    profiles: HashMap<String, ProfileConfig>
    
}

const NS: &str = "http://www.elektro-kapsel.se/audioplayer/v1";
type DynResult<T> = Result<T, Box<dyn Error + Send +Sync>>;
/*
fn get_text<I>(iter: &mut Peekable<I>) 
               -> DynResult<String>
    where I: Iterator<Item = XmlResult<XmlEvent>>

{
    match iter.peek() {
        Some(Ok(XmlEvent::Characters(_))) => {
            if let  Some(Ok(XmlEvent::Characters(text))) = iter.next() {
                return Ok(text);
            } else {
                panic!("Peek doesn't match next");
            }
        },
        Some(Err(_)) => {
            let e = iter.next().unwrap().unwrap_err();
            Err(e.into())
        },
        _ => Ok("".to_string())
    }
}

fn skip_node<I>(iter: &mut I) -> DynResult<()>
    where I: Iterator<Item = XmlResult<XmlEvent>>
{
    let mut level = 0;
    loop {
        match iter.next() {
            Some(Ok(XmlEvent::StartElement{..})) =>
            {
                level += 1;
            },
            Some(Ok(XmlEvent::EndElement{..})) =>
            {
                level -= 1;
                if level <= 0 {
                    break;
                }
            },
            Some(Ok(_)) =>
            {
                if level <= 0 {
                    break;
                }
                
            },
            Some(Err(e)) => return Err(e.into()),
            _ => break
        }
    }
    Ok(())
}
*/
fn expect_element<'a, I>(iter: &'a mut XmlSiblingIter<I>, elem_name: &str) -> DynResult<&'a Vec<OwnedAttribute>>
    where I: Iterator<Item = XmlResult<XmlEvent>>
{
    let ns_name = OwnedName{local_name: elem_name.to_string(),
                            namespace: Some(NS.to_string()),
                            prefix: None};
    match iter.current_node() {
        XmlEvent::StartElement{name,attributes, ..}
            if name == &ns_name => 
        {
            Ok(attributes)
        },
        XmlEvent::StartElement{
            name:OwnedName{local_name,..},..} => {
            Err(format!("Expected {}, found {}",
                        elem_name, local_name).into())
        },
        _ => Err(format!("Expected {}, no element found",
                         elem_name).into())
            
    }
   
}

fn parse_bind<I>(iter: &mut XmlSiblingIter<I>) -> DynResult<String>
where I: Iterator<Item = XmlResult<XmlEvent>>

{
    Ok(iter.get_text_content()?)
}                 


fn parse_file_clip<I>(iter: &mut XmlSiblingIter<I>) 
                      -> DynResult<(String,ClipType)>
where I: Iterator<Item = XmlResult<XmlEvent>>
{
    let [id] = get_attrs(iter.current_node(), &["id"], &None)?;
    let file_name = iter.get_text_content()?;
    Ok((id, ClipType::File(file_name)))
}

fn parse_clips<I>(parent: &mut XmlSiblingIter<I>) 
                  -> DynResult<HashMap<String, ClipType>>
where I: Iterator<Item = XmlResult<XmlEvent>>

{
    let mut clips = HashMap::new();
    let mut children = parent.child_iter()?;
    if let Some(XmlEvent::StartElement{name,..}) 
        = children.next_node().transpose()? {
            if let OwnedName{local_name, 
                             namespace: Some(name_ns), ..}= &name {
                if name_ns.as_str() == NS {
                    println!("Name: {}", name);
                    match local_name.as_str() {
                        "file" => {
                            let (id,clip) = parse_file_clip(&mut children)?;
                            clips.insert(id, clip);
                        },
                        _ => {
                            return Err(format!("Invalid node {}", 
                                               local_name).into())
                        }
                    }
                }
            }
        }
    Ok(clips)
}

fn parse_action<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<ActionType>
where I: Iterator<Item = XmlResult<XmlEvent>>
{
    let action;
    let node = parent.current_node();
    match node {
        XmlEvent::StartElement{name,..} =>
        {
            let OwnedName{local_name, 
                          namespace: name_ns, ..}= &name;
            if name_ns == &Some(NS.to_string()) {
                match local_name.as_str() {
                    "sequence" => {
                        action = parse_sequence(parent)?;
                    },
                    "play" => {
                        action = parse_play(parent)?;
                    },
                    "wait" => {
                        action = parse_wait(parent)?;
                    },
                    "alarm_restart" => {
                        action = ActionType::AlarmRestart;
                    },
                    "set_profile" => {
                        action = parse_set_profile(parent)?;
                    },
                    "repeat" => {
                        action = parse_repeat(parent)?;
                    },
                    _ => {
                        return Err(UnexpectedEvent(node.clone()).into())
                    }
                }
            } else {
                return Err(UnexpectedEvent(node.clone()).into())
            } 
        },
        _ev => return Err(invalid_state!().into())
    }
    Ok(action)
}

fn parse_play<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<ActionType>
where I: Iterator<Item = XmlResult<XmlEvent>>
{
    let sound = parent.get_text_content()?;
    Ok(ActionType::Play{sound})
}

fn parse_wait<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<ActionType>
where I: Iterator<Item = XmlResult<XmlEvent>>
{
    let time_str = parent.get_text_content()?;
    let time_str = time_str.trim();
    let (value_str, unit_str) = time_str.split_at(time_str.len() -1);
    let scale = match unit_str {
        "s" => 1.0,
        "m" => 60.0,
        "h" => 60.0*60.0,
        u => return Err(format!("Unknown time unit '{}'", u).into())
    };
    let value:f64 = value_str.trim().parse()?;
    Ok(ActionType::Wait(Duration::from_secs_f64(value*scale)))
}

fn parse_repeat<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<ActionType>
where I: Iterator<Item = XmlResult<XmlEvent>>
{
    let node = parent.current_node();
    let count_str = get_attr(node, "count", &None)?;
    let count;
    if let Some(count_str) = count_str {
        count = Some(count_str.parse()?);
    } else {
        count = None;
    }
    let action = parse_sequence(parent)?;
    Ok(ActionType::Repeat{count, action: Box::new(action)})
}

fn parse_set_profile<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<ActionType>
where I: Iterator<Item = XmlResult<XmlEvent>>
{
    let profile = parent.get_text_content()?;
    Ok(ActionType::SetProfile{profile})
}

fn parse_sequence<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<ActionType>
where I: Iterator<Item = XmlResult<XmlEvent>>
{
    let mut actions = Vec::new();
    let mut children = parent.child_iter()?;
    while children.next_node().transpose()?.is_some() {
        let action = parse_action(&mut children)?;
        actions.push(action);
    }
    if actions.is_empty() {
        return Err("No action in sequence".into())
    }
    if actions.len() == 1 {
        Ok(actions.pop().unwrap())
    } else {
        Ok(ActionType::Sequence(actions))
    }
}


fn parse_tag_trigger<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<(String, TagTriggerConfig)>
where I: Iterator<Item = XmlResult<XmlEvent>>
{
    let node = parent.current_node();
    let [tag_name] = get_attrs(node, &["tag"], &None)?;

    let trigger;

    
    match node {
        XmlEvent::StartElement{name: OwnedName{
            local_name, 
            namespace: Some(name_ns), 
            ..},..} =>
        {
            if name_ns.as_str() == NS {
                match local_name.as_str() {
                    "toggle" => {
                        trigger = TagTriggerType::Toggle;   
                    },
                    "equals" => {
                        trigger = TagTriggerType::Toggle;   
                    },
                    _ => {
                        return Err(UnexpectedEvent(node.clone())
                                   .into())
                    }
                }
            } else {
                return Err(UnexpectedEvent(node.clone()).into())
            }
        },
        _ => return Err(UnexpectedEvent(node.clone()).into())
    }
    
    let mut seq = parent.child_iter()?;
    let action = parse_sequence(&mut seq)?;
    Ok((tag_name, TagTriggerConfig{trigger, action}))
}
    
fn parse_tags<I>(parent: &mut XmlSiblingIter<I>) 
                 -> DynResult<HashMap<String, TagTriggerConfig>>
where I: Iterator<Item = XmlResult<XmlEvent>>
    
{
    let mut tags = HashMap::new();
    let mut children = parent.child_iter()?;
    while children.next_node().transpose()?.is_some() {
        let (tag,trigger) = parse_tag_trigger(&mut children)?;
        tags.insert(tag,trigger);   
    }
    Ok(tags)
}

fn parse_playback_device<I>(iter: &mut XmlSiblingIter<I>, player: &mut PlayerConfig)
                            -> DynResult<()>
where I: Iterator<Item = XmlResult<XmlEvent>>
{

    let [rate_str, channels_str] =
        get_attrs(iter.current_node(), &["rate", "channels"], &None)?; 
    player.rate = rate_str.parse()?;
    player.channels = channels_str.parse()?;



    player.playback_device = iter.get_text_content()?;

    Ok(())
}

pub fn read_file<R: Read>(source: R) -> DynResult<PlayerConfig>
{
    let parser_conf = ParserConfig::new()
        .trim_whitespace(true)
        .ignore_comments(true);
    let reader = parser_conf.create_reader(source);
    let mut top = TopElement::new(reader.into_iter())?;
    let mut node_iter = top.child_iter()?;
    let mut player = PlayerConfig {
        bind: "/tmp/siemens/automation/HmiRunTime".to_string(),
        playback_device: "".to_string(),
        rate: 44100,
        channels: 2,
        clip_root: String::new(),
        clips: HashMap::new(),
        tags: HashMap::new(),
        profiles: HashMap::new()
    };

    
    expect_element(&mut node_iter, "audioplayer")?;
    while let Some(node_res) =  node_iter.next_node() {
        let node = node_res?;
        match node {
            XmlEvent::StartElement{name,..} =>
            {
                if let OwnedName{local_name, 
                                 namespace: Some(name_ns), ..}= &name {
                    if name_ns.as_str() == NS {
                        println!("Name: {}", name);
                        match local_name.as_str() {
                            "bind" => {
                                player.bind = parse_bind(&mut node_iter)?;
                            },
                            "playback_device" => {
                                parse_playback_device(&mut node_iter,
                                                      &mut player)?;
                            },
                            "clips" => {
                                player.clips = parse_clips(&mut node_iter)?;
                            },
                            "tags" => {
                                player.tags = parse_tags(&mut node_iter)?;
                            },
                            "alarms" => {
                                //parse_alarms(&mut node_iter)?;
                            },
                            _ => {
                                return Err(format!("Invalid node {}", 
                                                   local_name).into())
                            }
                        }
                    }
                }
            },
            XmlEvent::Characters(text) => {
                 return Err(format!("Extra text found: \"{}\"", text).into())
            },
            _ => {}
        }
    }
    Ok(player)
}
    
#[test]
fn test_parser()
{
    let doc = r#"
<?xml version="1.0" encoding="UTF-8"?>
<audioplayer xmlns="http://www.elektro-kapsel.se/audioplayer/v1">
  <bind>/tmp/siemens/automation/HmiRunTime</bind>
  <playback_device rate="44100" channels="2">plughw:SoundBar</playback_device>
  <clips> 
    <file id="SoundAlarm">Alarm.wav</file>
    <file id="SoundInfo">Info.wav</file>
    <file id="SoundAccept">Knapp4.wav</file>
    <file id="SoundExe">Knapp2.wav</file>
    <file id="SoundInc">Knapp3.wav</file>
    <file id="SoundDec">Knapp4.wav</file>
  </clips>
  <tags>
    <toggle tag="SoundAlarm">
      <play>SoundAlarm</play>
    </toggle>
    <toggle tag="SoundInc">
      <play>SoundInfo</play>
    </toggle>
    <toggle tag="SoundExe">
      <play>SoundExe</play>
    </toggle>
    <toggle tag="SoundDec">
      <play>SoundInc</play>
    </toggle>
    <toggle tag="SoundDec">
      <play>SoundInc</play>
    </toggle>
    <toggle tag="AlarmRestart">
      <alarm_restart/>
    </toggle>
    <equals tag="AlarmProfile" value="0">
      <set_profile>Normal</set_profile>
    </equals>
    <equals tag="AlarmProfile" value="1">
      <set_profile>Operation</set_profile>
    </equals>
    <toggle tag="SequenceTest">
      <play>SoundExe</play>
      <wait>3s</wait>
      <repeat count="3">
        <play>SoundInc</play>
      </repeat>
    </toggle>
  </tags>
  <alarms>
    <filter id="AlarmsUnacked">
      <class>Larm</class>
      <unacked/>
    </filter>
    <filter id="AlarmsRaised">
      <class>Larm</class>
      <raised/>
    </filter>
    <filter id="Warnings">
      <class>Varning</class>
      <raised/>
    </filter>
    <sequence id="AlarmRepeat">
      <repeat>
	<repeat count="20">
	  <play>SoundAlarm</play>
	  <wait>5s</wait>
	</repeat>
	<wait>6h</wait>
      </repeat>
    </sequence>
    <sequence id="AlarmDelayed">
      <repeat>
	<wait>5m</wait>
	<repeat count="20">
	  <play>SoundAlarm</play>
	  <wait>5s</wait>
	</repeat>
      </repeat>
    </sequence>

    <sequence id="AlarmOnce">
      <play>SoundAlarm</play>
    </sequence>
    <sequence id="InfoOnce">
      <play>SoundInfo</play>
    </sequence>
    <profile id="Normal">
      <while filter="AlarmUnacked" state="not_empty">
	<sequence use="AlarmRepeat"/>
      </while>
      <while filter="AlarmsRaised" state="not_empty">
	<sequence use="AlarmDelayed"/>
      </while>
	
      <when filter="Varning" event="add">
	<run_sequence>InfoOnce</run_sequence>
      </when>
    </profile>
  </alarms>
</audioplayer>
"#;
    read_file(str::as_bytes(doc)).unwrap();
}
