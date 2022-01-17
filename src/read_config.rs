use crate::alarm_filter;
use crate::xml_stack::{TopElement, XmlSiblingIter};
use std::collections::HashMap;
use std::convert::TryInto;
use std::error::Error;
use std::io::Read;
use std::num::NonZeroU32;
use std::time::Duration;
use xml::attribute::OwnedAttribute;
use xml::name::OwnedName;
use xml::reader::Result as XmlResult;
use xml::reader::XmlEvent;
use xml::ParserConfig;
#[derive(Debug)]
pub enum ConfigError {
    UnexpectedEvent(XmlEvent),
    UnexpectedAttribute(String),
    MissingAttribute(String),
    InvalidState {
        file: String,
        line: u32,
        column: u32,
    },
}

use ConfigError::*;

impl std::error::Error for ConfigError {}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        match self {
            UnexpectedEvent(event) => match event {
                XmlEvent::StartElement { name, .. } => {
                    write!(f, "Unexpected start tag '{}'", name.local_name)
                }
                XmlEvent::EndElement { name } => {
                    write!(f, "Unexpected end tag '{}'", name.local_name)
                }
                XmlEvent::Characters(text) => write!(f, "Unexpected text '{}'", text),
                ev => write!(f, "Unexpected XML event '{:?}'", ev),
            },
            UnexpectedAttribute(name) => write!(f, "Unexpected attribute '{}'", name),
            MissingAttribute(name) => write!(f, "Missing attribute '{}'", name),
            InvalidState { file, line, column } => {
                write!(f, "Invalid state at {}:{}:{}", file, line, column)
            }
        }
    }
}

macro_rules! invalid_state {
    () => {
        InvalidState {
            file: file!().to_string(),
            line: line!(),
            column: column!(),
        }
    };
}

#[derive(Debug)]
enum AttrError {
    Missing {
        element: OwnedName,
        attribute: OwnedName,
    },
    //Unexpected{element: OwnedName, attribute: OwnedName},
    WrongEventType,
}

impl std::error::Error for AttrError {}

impl std::fmt::Display for AttrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        match self {
            AttrError::Missing { element, attribute } => write!(
                f,
                "Missing attribute '{}' on element '{}'",
                attribute.local_name, element.local_name
            ),
            /*
            AttrError::Unexpected{element,attribute} =>
                write!(f, "Unexpected attribute '{}' on element '{}'",
                       attribute.local_name, element.local_name),
             */
            AttrError::WrongEventType => {
                write!(f, "Can only read attributes from StartElement events")
            }
        }
    }
}

fn get_attrs<const N: usize>(
    event: &XmlEvent,
    attr_names: &[&str; N],
    namespace: &Option<String>,
) -> Result<[String; N], AttrError> {
    let attrs;
    let elem_name;
    if let XmlEvent::StartElement {
        name, attributes, ..
    } = event
    {
        attrs = attributes;
        elem_name = name;
    } else {
        return Err(AttrError::WrongEventType);
    }

    let mut out = Vec::with_capacity(N);
    for local in attr_names {
        if let Some(attr) = attrs
            .iter()
            .find(|a| local == &a.name.local_name && namespace == &a.name.namespace)
        {
            out.push(attr.value.clone());
        } else {
            return Err(AttrError::Missing {
                element: elem_name.clone(),
                attribute: OwnedName {
                    local_name: local.to_string(),
                    namespace: namespace.clone(),
                    prefix: None,
                },
            });
        }
    }
    Ok(out.try_into().unwrap())
}

fn get_attr<'a>(
    event: &'a XmlEvent,
    attr_name: &str,
    namespace: &Option<String>,
) -> Result<Option<&'a str>, AttrError> {
    let attrs;
    if let XmlEvent::StartElement { attributes, .. } = event {
        attrs = attributes;
    } else {
        return Err(AttrError::WrongEventType);
    }

    let out;
    if let Some(attr) = attrs
        .iter()
        .find(|a| attr_name == a.name.local_name && namespace == &a.name.namespace)
    {
        out = Some(attr.value.as_str());
    } else {
        out = None;
    }
    Ok(out)
}

// Call child_func for each child node. Stops immediately when an  error is returned.
fn children_for_each<F, I>(child_iter: &mut XmlSiblingIter<I>, mut child_func: F) -> DynResult<()>
where
    F: FnMut(&str, &mut XmlSiblingIter<I>) -> DynResult<()>,
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    // Loop over all children
    while let Some(node_res) = child_iter.next_node() {
        let node = node_res?; // Check for parse error
        match node {
            XmlEvent::StartElement {
                name:
                    OwnedName {
                        local_name,
                        namespace: Some(name_ns),
                        ..
                    },
                ..
            } => {
                let local_name = local_name.clone();
                if name_ns.as_str() == NS {
                    child_func(local_name.as_str(), child_iter)?;
                }
            }
            // Dont't allow text between element
            XmlEvent::Characters(text) => {
                return Err(format!("Extra text found: \"{}\"", text).into())
            }
            // Ignore everything else
            _ => {}
        }
    }
    Ok(())
}

#[derive(Debug)]
pub enum ClipType {
    File(String),
    Sine {
        amplitude: f64,
        frequency: f64,
        duration: Duration,
    },
}

#[derive(Debug, Clone)]
pub enum TagTriggerType {
    Toggle,
    Equals { value: i32 },
}

#[derive(Debug)]
pub struct TagTriggerConfig {
    pub trigger: TagTriggerType,
    pub action: ActionType,
}

#[derive(Debug)]
pub enum ActionType {
    Sequence(Vec<ActionType>),
    Parallel(Vec<ActionType>),
    Play {
        priority: i32,
        timeout: Option<Duration>,
        sound: String,
    },
    Wait(Duration),
    Reference(String),
    // No count means forever.
    Repeat {
        count: Option<NonZeroU32>,
        action: Box<ActionType>,
    },
    AlarmRestart,
    SetProfile {
        profile: String,
    },
}

#[derive(Debug)]
pub struct ProfileConfig {
    pub triggers: Vec<AlarmTriggerConfig>,
}

#[derive(Debug)]
pub struct PlayerConfig {
    pub bind: String,
    pub playback_device: String,
    pub rate: u32,
    pub channels: u8,
    pub clip_root: String,
    pub clips: HashMap<String, ClipType>,
    pub named_actions: Vec<(String, ActionType)>,
    pub tag_triggers: Vec<(String, TagTriggerConfig)>,
    pub named_alarm_filters: HashMap<String, alarm_filter::BoolOp>,
    pub alarm_profiles: HashMap<String, ProfileConfig>,
}

const NS: &str = "http://www.elektro-kapsel.se/audioplayer/v1";
type DynResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

fn expect_element<'a, I>(
    iter: &'a mut XmlSiblingIter<I>,
    elem_name: &str,
) -> DynResult<&'a Vec<OwnedAttribute>>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    let ns_name = OwnedName {
        local_name: elem_name.to_string(),
        namespace: Some(NS.to_string()),
        prefix: None,
    };
    match iter.current_node() {
        XmlEvent::StartElement {
            name, attributes, ..
        } if name == &ns_name => Ok(attributes),
        XmlEvent::StartElement {
            name: OwnedName { local_name, .. },
            ..
        } => Err(format!("Expected {}, found {}", elem_name, local_name).into()),
        _ => Err(format!("Expected {}, no element found", elem_name).into()),
    }
}

fn parse_bind<I>(iter: &mut XmlSiblingIter<I>) -> DynResult<String>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    Ok(iter.get_text_content()?)
}

fn parse_file_clip<I>(iter: &mut XmlSiblingIter<I>) -> DynResult<(String, ClipType)>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    let [id] = get_attrs(iter.current_node(), &["id"], &None)?;
    let file_name = iter.get_text_content()?;
    Ok((id, ClipType::File(file_name)))
}

fn parse_sine_clip<I>(iter: &mut XmlSiblingIter<I>) -> DynResult<(String, ClipType)>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    let [id, amp_str, freq_str, dur_str] = get_attrs(
        iter.current_node(),
        &["id", "amplitude", "frequency", "duration"],
        &None,
    )?;
    let amplitude = str::parse(&amp_str).map_err(|_e| "Failed to parse amplitude value")?;
    let frequency = str::parse(&freq_str).map_err(|_e| "Failed to parse frequency value")?;
    let duration = parse_duration(&dur_str)?;
    Ok((
        id,
        ClipType::Sine {
            amplitude,
            frequency,
            duration,
        },
    ))
}

fn parse_clips<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<HashMap<String, ClipType>>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    let mut clips = HashMap::new();
    children_for_each(&mut parent.child_iter()?, |local_name: &str, child_iter| {
        match local_name {
            "file" => {
                let (id, clip) = parse_file_clip(child_iter)?;
                clips.insert(id, clip);
            }
            "sine" => {
                let (id, clip) = parse_sine_clip(child_iter)?;
                clips.insert(id, clip);
            }
            _ => return Err(format!("Invalid node {}", local_name).into()),
        }
        Ok(())
    })?;
    Ok(clips)
}

fn parse_action<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<ActionType>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    let action;
    let node = parent.current_node();
    match node {
        XmlEvent::StartElement { name, .. } => {
            let OwnedName {
                local_name,
                namespace: name_ns,
                ..
            } = &name;
            if name_ns == &Some(NS.to_string()) {
                match local_name.as_str() {
                    "sequence" => {
                        action = parse_sequence(parent)?;
                    }
                    "parallel" => {
                        action = parse_parallel(parent)?;
                    }
                    "play" => {
                        action = parse_play(parent)?;
                    }
                    "wait" => {
                        action = parse_wait(parent)?;
                    }
                    "alarm_restart" => {
                        action = ActionType::AlarmRestart;
                    }
                    "set_profile" => {
                        action = parse_set_profile(parent)?;
                    }
                    "repeat" => {
                        action = parse_repeat(parent)?;
                    }
                    "action" => {
                        action = parse_action_ref(parent)?;
                    }
                    _ => return Err(UnexpectedEvent(node.clone()).into()),
                }
            } else {
                return Err(UnexpectedEvent(node.clone()).into());
            }
        }
        _ev => return Err(invalid_state!().into()),
    }
    Ok(action)
}

fn parse_play<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<ActionType>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    let priority_str = get_attr(parent.current_node(), "priority", &None)?;
    let priority = if let Some(s) = priority_str {
        s.parse()?
    } else {
        0 // Default priority
    };
    let timeout_str = get_attr(parent.current_node(), "timeout", &None)?;
    let timeout = timeout_str.map_or(Ok(None), |s| Some(parse_duration(&s)).transpose())?;
    let sound = parent.get_text_content()?;
    Ok(ActionType::Play {
        priority,
        timeout,
        sound,
    })
}

fn parse_duration(time_str: &str) -> DynResult<Duration> {
    let time_str = time_str.trim();
    let (value_str, unit_str) = time_str.split_at(time_str.len() - 1);
    let value: f64 = value_str.trim().parse()?;
    let scale = match unit_str {
        "s" => 1.0,
        "m" => 60.0,
        "h" => 60.0 * 60.0,
        u => return Err(format!("Unknown time unit '{}'", u).into()),
    };
    Ok(Duration::from_secs_f64(value * scale))
}

fn parse_wait<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<ActionType>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    let time_str = parent.get_text_content()?;

    Ok(ActionType::Wait(parse_duration(&time_str)?))
}

fn parse_repeat<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<ActionType>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
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
    Ok(ActionType::Repeat {
        count,
        action: Box::new(action),
    })
}

fn parse_set_profile<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<ActionType>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    let profile = parent.get_text_content()?;
    Ok(ActionType::SetProfile { profile })
}

fn parse_sequence<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<ActionType>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    let mut actions = Vec::new();
    let mut children = parent.child_iter()?;
    while children.next_node().transpose()?.is_some() {
        let action = parse_action(&mut children)?;
        actions.push(action);
    }
    if actions.is_empty() {
        return Err("No action in sequence".into());
    }
    if actions.len() == 1 {
        Ok(actions.pop().unwrap())
    } else {
        Ok(ActionType::Sequence(actions))
    }
}

fn parse_parallel<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<ActionType>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    let mut actions = Vec::new();
    let mut children = parent.child_iter()?;
    while children.next_node().transpose()?.is_some() {
        let action = parse_action(&mut children)?;
        actions.push(action);
    }
    if actions.is_empty() {
        return Err("No action in parallel".into());
    }
    if actions.len() == 1 {
        Ok(actions.pop().unwrap())
    } else {
        Ok(ActionType::Parallel(actions))
    }
}

fn parse_action_ref<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<ActionType>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    let node = parent.current_node();
    let action_ref =
        get_attr(node, "use", &None)?.ok_or("Action element must hav use attribute")?;

    Ok(ActionType::Reference(action_ref.to_owned()))
}

fn parse_actions<I>(
    parent: &mut XmlSiblingIter<I>,
    actions: &mut Vec<(String, ActionType)>,
) -> DynResult<()>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    let mut children = parent.child_iter()?;
    while children.next_node().transpose()?.is_some() {
        let id = get_attr(children.current_node(), "id", &None)?
            .ok_or("Action must have an id")?
            .to_owned();
        let action = parse_action(&mut children)?;
        actions.push((id, action));
    }
    Ok(())
}

fn parse_tags<I>(parent: &mut XmlSiblingIter<I>) -> DynResult<Vec<(String, TagTriggerConfig)>>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    let mut tags = Vec::new();
    children_for_each(&mut parent.child_iter()?, |local_name, child_iter| {
        let node = child_iter.current_node();
        let [tag_name] = get_attrs(node, &["tag"], &None)?;
        let trigger;
        match local_name {
            "toggle" => {
                trigger = TagTriggerType::Toggle;
            }
            "equals" => {
                let [value_str] = get_attrs(node, &["value"], &None)?;
                let value = value_str.parse()?;
                trigger = TagTriggerType::Equals { value };
            }
            _ => return Err(UnexpectedEvent(node.clone()).into()),
        }
        let mut seq = child_iter.child_iter()?;
        let action = parse_sequence(&mut seq)?;
        tags.push((tag_name, TagTriggerConfig { trigger, action }));
        Ok(())
    })?;
    Ok(tags)
}


#[derive(Debug,Copy, Clone)]
pub enum AlarmTriggerType {
    WhileAnyActive,
    WhileNoneActive,
    WhenRaised,
    WhenFirstRaised,
    WhenCleared,
    WhenLastCleared,
}

#[derive(Debug)]
pub struct AlarmTriggerConfig {
    pub trigger_type: AlarmTriggerType,
    pub filter_id: String,
    pub action: ActionType,
}

fn parse_alarm_profile<I>(
    parent: &mut XmlSiblingIter<I>,

) -> DynResult<ProfileConfig>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    let mut triggers: Vec<AlarmTriggerConfig> = Vec::new();
    children_for_each(&mut parent.child_iter()?, |local_name: &str, child| {
        let node = child.current_node();
        match local_name {
            "while" => {
                let filter_id = get_attr(node, "filter", &None)?
                    .ok_or("while must have filter_id attribute")?
                    .to_string();
                let state = get_attr(node, "active", &None)?;
                let trigger_type = match state {
                    Some("none") => AlarmTriggerType::WhileNoneActive,
                    Some("any") => AlarmTriggerType::WhileAnyActive,
                    Some(_) => return Err("Invalid active attribute value for while".into()),
                    None => AlarmTriggerType::WhileAnyActive,
                };
                let action = parse_sequence(child)?;
                triggers.push(AlarmTriggerConfig {
                    trigger_type,
                    filter_id,
                    action,
                });
                Ok(())
            }
            "when" => {
                let filter_id = get_attr(node, "filter", &None)?
                    .ok_or("when must have filter_id attribute")?
                    .to_string();
                let event = get_attr(node, "event", &None)?;
                let trigger_type = match event {
                    Some("raised") => AlarmTriggerType::WhenRaised,
                    Some("first_raised") => AlarmTriggerType::WhenFirstRaised,
                    Some("cleared") => AlarmTriggerType::WhenCleared,
                    Some("last_cleared") => AlarmTriggerType::WhenLastCleared,
                    Some(_) => return Err("Invalid event attribute value for when".into()),
                    None => AlarmTriggerType::WhenRaised,
                };
                let action = parse_sequence(child)?;
                triggers.push(AlarmTriggerConfig {
                    trigger_type,
                    filter_id,
                    action,
                });

                Ok(())
            }
            _ => return Err(UnexpectedEvent(child.current_node().clone()).into()),
        }
    })?;
    let profile = ProfileConfig{triggers};
    Ok(profile)
}
fn parse_alarms<I>(
    parent: &mut XmlSiblingIter<I>,
    named_filters: &mut HashMap<String, alarm_filter::BoolOp>,
    profiles: &mut HashMap<String,ProfileConfig>,
) -> DynResult<()>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    children_for_each(&mut parent.child_iter()?, |local_name: &str, child| {
        match local_name {
            "filter" => {
                let node = child.current_node();
                let [filter_id] = get_attrs(node, &["id"], &None)?;
                let filter_def = child.get_text_content()?;
                let op = match alarm_filter::parse_filter(&filter_def) {
                    Ok(op) => op,
                    Err(e) => return Err(format!("Failed to parse alarm filter: {}", e).into()),
                };
                named_filters.insert(filter_id, op);
            }
            "profile" => {
		let node = child.current_node();
		let [name] = get_attrs(node, &["id"], &None)?;
                let profile = parse_alarm_profile(child)?;
		profiles.insert(name, profile);
            }
            _ => return Err(UnexpectedEvent(child.current_node().clone()).into()),
        }
        Ok(())
    })?;
    
    Ok(())
}

fn parse_playback_device<I>(
    iter: &mut XmlSiblingIter<I>,
    player: &mut PlayerConfig,
) -> DynResult<()>
where
    I: Iterator<Item = XmlResult<XmlEvent>>,
{
    let [rate_str, channels_str] = get_attrs(iter.current_node(), &["rate", "channels"], &None)?;
    player.rate = rate_str.parse()?;
    player.channels = channels_str.parse()?;

    player.playback_device = iter.get_text_content()?;

    Ok(())
}

pub fn read_file<R: Read>(source: R) -> DynResult<PlayerConfig> {
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
        named_actions: Vec::new(),
        tag_triggers: Vec::new(),
	named_alarm_filters: HashMap::new(),
        alarm_profiles: HashMap::new(),
    };

    expect_element(&mut node_iter, "audioplayer")?;
    children_for_each(&mut node_iter, |local_name, node_iter| {
        match local_name {
            "bind" => {
                player.bind = parse_bind(node_iter)?;
            }
            "playback_device" => {
                parse_playback_device(node_iter, &mut player)?;
            }
            "clips" => {
                if let Some(path) = get_attr(node_iter.current_node(), "path", &None)? {
                    player.clip_root = path.to_string();
                }
                player.clips = parse_clips(node_iter)?;
            }
            "actions" => {
                parse_actions(node_iter, &mut player.named_actions)?;
            }
            "tags" => {
                player.tag_triggers = parse_tags(node_iter)?;
            }
            "alarms" => {
                parse_alarms(
                    node_iter,
                    &mut player.named_alarm_filters,
                    &mut player.alarm_profiles,
                )?;
            }
            _ => return Err(format!("Invalid node {}", local_name).into()),
        }
        Ok(())
    })?;
    Ok(player)
}

#[test]
fn test_parser() {
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
  <actions>
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
  </actions> 
  <alarms>
    <filter id="AlarmsUnacked">
      AlarmClassName = 'Larm' AND State = 'in' AND State = 'in out'
    </filter>
    <filter id="AlarmsRaised">
      AlarmClassName = 'Larm' AND State = 'in' AND State = 'in ack'
    </filter>
    <filter id="Warnings">
      AlarmClassName = 'Varning' AND State = 'in' AND State = 'in ack'
    </filter>
      
    <profile id="Normal">
      <while filter="AlarmUnacked">
	<action use="AlarmRepeat"/>
      </while>
      <while filter="AlarmsRaised">
	<action use="AlarmDelayed"/>
      </while>
	
      <when filter="Varning" event="raised">
	<action use="AlarmDelayed"/>
      </when>
    </profile>
  </alarms>
</audioplayer>
"#;
    read_file(str::as_bytes(doc)).unwrap();
}
