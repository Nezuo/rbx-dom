use std::io::Read;

use failure::Fail;
use log::trace;
use xml::{
    attribute::OwnedAttribute,
    reader::{self, ParserConfig},
};

use crate::{
    error::{DecodeError as NewDecodeError, DecodeErrorKind},
};

pub(crate) use xml::reader::XmlEvent as XmlReadEvent;
pub(crate) use xml::reader::Error as XmlReadError;
pub(crate) type XmlReadResult = Result<XmlReadEvent, XmlReadError>;

/// Indicates an error trying to parse an rbxmx or rbxlx document
#[derive(Debug, Fail, Clone, PartialEq)]
pub enum DecodeError {
    #[fail(display = "XML read error: {}", _0)]
    XmlError(#[fail(cause)] reader::Error),

    #[fail(display = "Float parse error: {}", _0)]
    ParseFloatError(#[fail(cause)] std::num::ParseFloatError),

    #[fail(display = "Int parse error: {}", _0)]
    ParseIntError(#[fail(cause)] std::num::ParseIntError),

    #[fail(display = "Base64 decode error: {}", _0)]
    DecodeBase64Error(#[fail(cause)] base64::DecodeError),

    // TODO: Switch to Cow<'static, str>?
    #[fail(display = "{}", _0)]
    Message(&'static str),

    #[fail(display = "Malformed document")]
    MalformedDocument,

    #[fail(display = "{}", _0)]
    New(#[fail(cause)] NewDecodeError),

    #[doc(hidden)]
    #[fail(display = "<this variant should never exist>")]
    __Nonexhaustive,
}

impl From<reader::Error> for DecodeError {
    fn from(error: reader::Error) -> DecodeError {
        DecodeError::XmlError(error)
    }
}

impl From<std::num::ParseFloatError> for DecodeError {
    fn from(error: std::num::ParseFloatError) -> DecodeError {
        DecodeError::ParseFloatError(error)
    }
}

impl From<std::num::ParseIntError> for DecodeError {
    fn from(error: std::num::ParseIntError) -> DecodeError {
        DecodeError::ParseIntError(error)
    }
}

impl From<base64::DecodeError> for DecodeError {
    fn from(error: base64::DecodeError) -> DecodeError {
        DecodeError::DecodeBase64Error(error)
    }
}

impl From<NewDecodeError> for DecodeError {
    fn from(error: NewDecodeError) -> DecodeError {
        DecodeError::New(error)
    }
}

/// A wrapper around an XML event iterator created by xml-rs.
pub struct XmlEventReader<R: Read> {
    reader: xml::EventReader<R>,
    peeked: Option<Result<XmlReadEvent, xml::reader::Error>>,
    finished: bool,
}

impl<R: Read> Iterator for XmlEventReader<R> {
    type Item = XmlReadResult;

    fn next(&mut self) -> Option<XmlReadResult> {
        if let Some(value) = self.peeked.take() {
            return Some(value);
        }

        if self.finished {
            return None;
        }

        loop {
            match self.reader.next() {
                Ok(item) => match item {
                    XmlReadEvent::Whitespace(_) => continue,
                    XmlReadEvent::EndDocument => {
                        self.finished = true;
                        return Some(Ok(item))
                    },
                    _ => return Some(Ok(item))
                },
                Err(err) => {
                    self.finished = true;
                    return Some(Err(err))
                }
            }
        }
    }
}

impl<R: Read> XmlEventReader<R> {
    /// Constructs a new `XmlEventReader` from a source that implements `Read`.
    pub fn from_source(source: R) -> XmlEventReader<R> {
        let reader = ParserConfig::new()
            .ignore_comments(true)
            .create_reader(source);

        XmlEventReader {
            reader,
            peeked: None,
            finished: false,
        }
    }

    /// Borrows the next element from the event stream without consuming it.
    pub fn peek(&mut self) -> Option<&XmlReadResult> {
        if self.peeked.is_some() {
            return self.peeked.as_ref();
        }

        self.peeked = self.next();
        self.peeked.as_ref()
    }

    pub(crate) fn error<T: Into<DecodeErrorKind>>(&self, kind: T) -> NewDecodeError {
        NewDecodeError::new_from_reader(kind.into(), &self.reader)
    }

    pub fn expect_next(&mut self) -> Result<XmlReadEvent, NewDecodeError> {
        match self.next() {
            Some(Ok(event)) => Ok(event),
            Some(Err(err)) => Err(self.error(err)),
            None => Err(self.error(DecodeErrorKind::UnexpectedEof)),
        }
    }

    pub fn expect_peek(&mut self) -> Result<&XmlReadEvent, NewDecodeError> {
        // This weird transmute is here because NLL in current Rust (1.34)
        // extends borrows to the entire function when returning borrowed
        // values.
        //
        // This code without the transmute compiles with -Zpolonius as of
        // 2019-04-30. I don't believe it to be a soundness hole, but I also
        // don't fully understand why this transmute tricks Rust into thinking
        // the code is correct.
        let peeked_value = unsafe {
            std::mem::transmute::<
                Option<&Result<XmlReadEvent, XmlReadError>>,
                Option<&Result<XmlReadEvent, XmlReadError>>,
            >(self.peek())
        };

        match peeked_value {
            Some(Ok(event)) => Ok(event),
            Some(Err(_)) => Err(self.expect_next().unwrap_err()),
            None => Err(self.error(DecodeErrorKind::UnexpectedEof)),
        }
    }

    /// Consumes the next event and returns `Ok(())` if it was an opening tag
    /// with the given name, otherwise returns an error.
    pub fn expect_start_with_name(&mut self, expected_name: &str) -> Result<Vec<OwnedAttribute>, NewDecodeError> {
        match self.expect_next()? {
            XmlReadEvent::StartElement { name, attributes, namespace } => {
                if name.local_name != expected_name {
                    let event = XmlReadEvent::StartElement { name, attributes, namespace };
                    return Err(self.error(DecodeErrorKind::UnexpectedXmlEvent(event)));
                }

                Ok(attributes)
            }
            event => Err(self.error(DecodeErrorKind::UnexpectedXmlEvent(event)))
        }
    }

    /// Consumes the next event and returns `Ok(())` if it was a closing tag
    /// with the given name, otherwise returns an error.
    pub fn expect_end_with_name(&mut self, expected_name: &str) -> Result<(), NewDecodeError> {
        let event = self.expect_next()?;

        match &event {
            XmlReadEvent::EndElement { name, .. } => {
                if name.local_name != expected_name {
                    return Err(self.error(DecodeErrorKind::UnexpectedXmlEvent(event)));
                }

                Ok(())
            }
            _ => Err(self.error(DecodeErrorKind::UnexpectedXmlEvent(event)))
        }
    }

    /// Reads one `Characters` or `CData` event if the next event is a
    /// `Characters` or `CData` event.
    ///
    /// If the next event in the stream is not a character event, this function
    /// will return `Ok(None)` and leave the stream untouched.
    ///
    /// This is the inner kernel of `read_characters`, which is the public
    /// version of a similar idea.
    fn read_one_characters_event(&mut self) -> Result<Option<String>, NewDecodeError> {
        // This pattern (peek + next) is pretty gnarly but is useful for looking
        // ahead without touching the stream.

        match self.peek() {
            // If the next event is a `Characters` or `CData` event, we need to
            // use `next` to take ownership over it (with some careful unwraps)
            // and extract the data out of it.
            //
            // We could also clone the borrowed data obtained from peek, but
            // some of the character events can contain several megabytes of
            // data, so a copy is really expensive.
            Some(Ok(XmlReadEvent::Characters(_))) | Some(Ok(XmlReadEvent::CData(_))) => {
                match self.next().unwrap().unwrap() {
                    XmlReadEvent::Characters(value) | XmlReadEvent::CData(value) => Ok(Some(value)),
                    _ => unreachable!()
                }
            }

            // Since we can't use `?` (we have a `&Result` instead of a `Result`)
            // we have to do something similar to what it would do.
            Some(Err(_)) => {
                let kind = self.next().unwrap().unwrap_err();
                Err(self.error(kind))
            }

            None | Some(Ok(_)) => Ok(None),
        }
    }

    /// Reads a contiguous sequence of zero or more `Characters` and `CData`
    /// events from the event stream.
    ///
    /// Normally, consumers of xml-rs shouldn't need to do this since the
    /// combination of `cdata_to_characters` and `coalesce_characters` does
    /// something very similar. Because we want to support CDATA sequences that
    /// contain only whitespace, we have two options:
    ///
    /// 1. Every time we want to read an XML event, use a loop and skip over all
    ///    `Whitespace` events
    ///
    /// 2. Turn off `cdata_to_characters` in `ParserConfig` and use a regular
    ///    iterator filter to strip `Whitespace` events
    ///
    /// For complexity, performance, and correctness reasons, we switched from
    /// #1 to #2. However, this means we need to coalesce `Characters` and
    /// `CData` events ourselves.
    pub fn read_characters(&mut self) -> Result<String, NewDecodeError> {
        let mut buffer = match self.read_one_characters_event()? {
            Some(buffer) => buffer,
            None => return Ok(String::new()),
        };

        loop {
            match self.read_one_characters_event()? {
                Some(piece) => buffer.push_str(&piece),
                None => break,
            }
        }

        Ok(buffer)
    }

    /// Reads a tag completely and returns its text content. This is intended
    /// for parsing simple tags where we don't care about the attributes or
    /// children, only the text value, for Vector3s and such, which are encoded
    /// like:
    ///
    /// <Vector3>
    ///     <X>0</X>
    ///     <Y>0</Y>
    ///     <Z>0</Z>
    /// </Vector3>
    pub fn read_tag_contents(&mut self, expected_name: &str) -> Result<String, NewDecodeError> {
        self.expect_start_with_name(expected_name)?;
        let contents = self.read_characters()?;
        self.expect_end_with_name(expected_name)?;

        Ok(contents)
    }

    /// Consume events from the iterator until we reach the end of the next tag.
    pub fn eat_unknown_tag(&mut self) -> Result<(), NewDecodeError> {
        let mut depth = 0;

        trace!("Starting unknown block");

        loop {
            match self.expect_next()? {
                XmlReadEvent::StartElement { name, .. } => {
                    trace!("Eat unknown start: {:?}", name);
                    depth += 1;
                },
                XmlReadEvent::EndElement { name } => {
                    trace!("Eat unknown end: {:?}", name);
                    depth -= 1;

                    if depth == 0 {
                        trace!("Reached end of unknown block");
                        break;
                    }
                },
                other => {
                    trace!("Eat unknown: {:?}", other);
                },
            }
        }

        Ok(())
    }
}