use xml::reader::XmlEvent;
use xml::name::OwnedName;
use xml::reader::Result as XmlResult;
use xml::reader::Error as XmlError;

#[derive(Debug)]
pub enum Error
{
    ReaderError(Box<XmlError>),
    UnmatchedEndElement,
    NoEndElement,
    EndOfDocument,
    UnhandledNodes,
    StateError,
}

impl std::error::Error for Error {}

impl From<XmlError> for Error
{
    fn from(err: XmlError) -> Error
    {
        Error::ReaderError(Box::new(err))
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>)
           -> std::result::Result<(), std::fmt::Error>
    {
        match self {
            Error::ReaderError(e) => e.fmt(f),
            Error::UnmatchedEndElement =>
                write!(f,"End element doesn't match start element"),
            Error::NoEndElement =>
                write!(f,"End of element not found"),
            Error::EndOfDocument =>
                write!(f,"Unexpected end of document"),
            Error::UnhandledNodes =>
                write!(f,"Not all child node where handled"),
            Error::StateError =>
                write!(f,"The iterator is in an invalid state"),
        }            
    }
}
    

pub type Result<T> = std::result::Result<T, Error>;

struct ParsePosition<I>
where I: Iterator<Item = XmlResult<XmlEvent>>
{
    iter: I,
    current_event: XmlEvent,
    level: u32,
}

impl<I> ParsePosition<I> 
where I: Iterator<Item = XmlResult<XmlEvent>>
{
    fn next<'a>(&'a mut self) -> Result<&'a XmlEvent>
    {
        match self.current_event {
            XmlEvent::StartElement{..} => {
                self.level += 1;
            },
            _ => {}
        }
        match self.iter.next() {
            Some(Ok(event)) => {
                match event {
                    XmlEvent::EndElement{..} => {
                        self.level -= 1;
                    },
                    _ => {}
                }
                self.current_event = event;
                //println!("Next: {:?} @ {}", self.current_event, self.level);
                Ok(&self.current_event)
            },
            Some(Err(e)) => Err(e.into()),
            None => Err(Error::EndOfDocument)
        }
    }
}
    
pub struct TopElement<I>
where I: Iterator<Item = XmlResult<XmlEvent>>
{
    pos: ParsePosition<I>
}

pub struct XmlSiblingIter<'a,I>
where I: Iterator<Item = XmlResult<XmlEvent>>
{
    pos: &'a mut ParsePosition<I>,
    // Name of parent when iterating through children
    parent_name: Option<OwnedName>,
    level: u32
}

impl<I> TopElement<I> 
where I: Iterator<Item = XmlResult<XmlEvent>>
{
    pub fn new(mut iter: I) -> Result<TopElement<I>>
    {
        let event = loop {
            match iter.next() {
                Some(Ok(event)) => {
                    if let XmlEvent::StartElement{..} = event {
                        break event;
                    }
                },
                Some(Err(e)) => return Err(e.into()),
                None => return Err(Error::EndOfDocument)
            }
        };
        Ok(TopElement {
            pos: ParsePosition {
                iter,
                current_event: event,
                level: 0
            }
        })
    }

    pub fn child_iter<'b>(&'b mut self) -> Result<XmlSiblingIter<'b, I>>
    {
        Ok(XmlSiblingIter{pos: &mut self.pos,
                          parent_name: None,
                          level: 1})
    }
    
}

// The iterator must point at the start element, i.e. pos.next() gets
// the first child node

fn skip_children<'a, I>(pos: &'a mut ParsePosition<I>) -> Result<()>
where I: Iterator<Item = XmlResult<XmlEvent>>
{
    let start_name;
    if let XmlEvent::StartElement{name, ..} = &pos.current_event {
        start_name = name.clone();
    } else {
        return Err(Error::StateError);
    }
    let end_level = pos.level;
    loop {
        match pos.next() {
            Ok(XmlEvent::EndElement{name}) =>
            {
                if name == &start_name {
                    if end_level >= pos.level {
                        return Ok(())
                    }
                } else {
                    return Err(Error::UnmatchedEndElement)
                }
            },
            Ok(_) =>
            {
                if end_level >= pos.level {
                    return Err(Error::NoEndElement)
                }
                
            },
            Err(e) => return Err(e.into())
        }
    }
}


impl<'a, I> XmlSiblingIter<'a, I>
where I: Iterator<Item = XmlResult<XmlEvent>>
{
    
    pub fn next_node(&mut self) -> Option<Result<&XmlEvent>>
    {
        match self.pos.next() {
            Ok(_) => {}
            Err(e) => return Some(Err(e.into()))
        }

        if self.level < self.pos.level {
            loop {
                match self.pos.next() {
                    Ok(_) => {}
                    Err(e) => return Some(Err(e.into()))
                }
                if self.level == self.pos.level {
                    break
                }
            }
            if let Some(parent_name) = &self.parent_name {
                match &self.pos.current_event {
                    XmlEvent::EndElement{name} => {
                        if name != parent_name {
                            return Some(Err(Error::UnmatchedEndElement))
                        }
                    },
                    _ => return Some(Err(Error::StateError))
                }
            } else {
                return Some(Err(Error::StateError))
            }
            self.parent_name = None;
        }
        
        if let XmlEvent::EndElement{..} = &self.pos.current_event {
            if self.level > self.pos.level {
                return None
            } else {
                match self.pos.next() {
                    Ok(_) => {}
                        Err(e) => return Some(Err(e.into()))
                }
                if self.level > self.pos.level {
                    return None
                }
            }
        }
        if let XmlEvent::StartElement{name, ..} = &self.pos.current_event
        {
            self.parent_name = Some(name.clone());
        }
                    
        Some(Ok(&self.pos.current_event))
    }
    
    pub fn current_node(&self) -> &XmlEvent
    {
        &self.pos.current_event
    }
    
    pub fn child_iter<'b>(&'b mut self) -> Result<XmlSiblingIter<'b, I>>
    {
        if let XmlEvent::StartElement{name, ..} = &self.pos.current_event {
            self.parent_name = Some(name.clone());
        } else {
            return Err(Error::StateError)
        }
        self.level = self.pos.level;
        Ok(XmlSiblingIter{pos: &mut self.pos,
                          parent_name: None,
                          level: self.level + 1})
    }

    /// Traverse the sub tree and combine all text nodes
    pub fn get_text_content(&mut self) -> Result<String>
    {
        let mut text = String::new();
        let start_name;
        if let XmlEvent::StartElement{name, ..} = &self.pos.current_event {
            start_name = name.clone();
        } else {
            return Err(Error::StateError);
        }
        let end_level = self.pos.level;
        loop {
            match self.pos.next() {
                Ok(_) => {},
                Err(e) => return Err(e)
            };
            match &self.pos.current_event {
                XmlEvent::EndElement{name} =>
                {
                    if end_level >= self.pos.level {
                        if name == &start_name {
                            return Ok(text)
                        } else {    
                            return Err(Error::UnmatchedEndElement)
                        } 
                    }
                },
                
                // End on first node at the start level that is not an end node
                XmlEvent::Characters(str) => {
                    text += &str;
                },
                _ =>
                {
                    if end_level >= self.pos.level {
                        return Err(Error::NoEndElement)
                    }
                }
            }
        }
    }
}

#[cfg(test)]
use xml::reader::ParserConfig;

#[test]
fn test_sibling_iter()
{
      let doc = r#"
<?xml version="1.0" encoding="UTF-8"?>
<top xmlns="http://www.example.ex/test">
sjkj
<l1/>
<l2>
<l2_1/>
</l2>
<l3>
dasjkljjk
<l3_1>
<l3_1_1>
<l3_1_1_1/>
</l3_1_1>
</l3_1>
<l3_2/>
<l3_3>jlkjljk</l3_3>
</l3>
<l4>jkljkl</l4>
</top>
"#;
     let parser_conf = ParserConfig::new()
        .trim_whitespace(false)
        .ignore_comments(false);
    let reader = parser_conf.create_reader(str::as_bytes(doc));
    let mut event_iter = reader.into_iter();
    let mut top = TopElement::new(&mut event_iter).unwrap();
    let mut parent1 = top.child_iter().unwrap();
    while let Some(node) = parent1.next_node() {
        let node = node.unwrap();
        println!("Node: {:?}", node);
    }

}

#[cfg(test)]
const SPACES: &str = "                              ";

#[cfg(test)]
fn parse_child<'a, I>(mut iter: XmlSiblingIter<'a,I>, indent: usize)
    where I: Iterator<Item = XmlResult<XmlEvent>>
{
    while let Some(node) = iter.next_node() {
        let node = node.unwrap();
        println!("{}{:?}",&SPACES[0..indent], node);
        match node {
            XmlEvent::StartElement{..} => {
                let children = iter.child_iter().unwrap();
                parse_child(children, indent+2);
            },
            _ => {}
        }
    }
}

#[test]
fn test_recursion()
{
      let doc = r#"
<?xml version="1.0" encoding="UTF-8"?>
<top xmlns="http://www.example.ex/test">
sjkj
<l1/>
<l2>
<l2_1/>
</l2>
<l3>
dasjkljjk
<l3_1>
<l3_1_1>
<l3_1_1_1/>
</l3_1_1>
</l3_1>
<l3_2/>
<l3_3>jlkjljk</l3_3>
</l3>
<l4>jkljkl</l4>
</top>
"#;
     let parser_conf = ParserConfig::new()
        .trim_whitespace(true)
        .ignore_comments(false);
    let reader = parser_conf.create_reader(str::as_bytes(doc));
    let mut event_iter = reader.into_iter();
    let mut top = TopElement::new(&mut event_iter).unwrap();
    let mut children = top.child_iter().unwrap();
    parse_child(children, 2);
    
}

