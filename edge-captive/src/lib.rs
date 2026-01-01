#![cfg_attr(not(feature = "std"), no_std)]
#![warn(clippy::large_futures)]
#![allow(clippy::uninlined_format_args)]
#![allow(unknown_lints)]

use core::fmt::Display;
use core::time::Duration;

use domain::base::wire::Composer;
use domain::dep::octseq::{OctetsBuilder, Truncate};

use domain::{
    base::{
        iana::{Class, Opcode, Rcode},
        message::ShortMessage,
        message_builder::PushError,
        record::Ttl,
        wire::ParseError,
        Record, Rtype,
    },
    dep::octseq::ShortBuf,
    rdata::A,
};

// This mod MUST go first, so that the others see its macros.
pub(crate) mod fmt;

#[cfg(feature = "io")]
pub mod io;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum DnsError {
    ShortBuf,
    InvalidMessage,
}

impl Display for DnsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShortBuf => write!(f, "ShortBuf"),
            Self::InvalidMessage => write!(f, "InvalidMessage"),
        }
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for DnsError {
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            Self::ShortBuf => defmt::write!(f, "ShortBuf"),
            Self::InvalidMessage => defmt::write!(f, "InvalidMessage"),
        }
    }
}

impl core::error::Error for DnsError {}

impl From<ShortBuf> for DnsError {
    fn from(_: ShortBuf) -> Self {
        Self::ShortBuf
    }
}

impl From<PushError> for DnsError {
    fn from(_: PushError) -> Self {
        Self::ShortBuf
    }
}

impl From<ShortMessage> for DnsError {
    fn from(_: ShortMessage) -> Self {
        Self::InvalidMessage
    }
}

impl From<ParseError> for DnsError {
    fn from(_: ParseError) -> Self {
        Self::InvalidMessage
    }
}

pub fn reply(
    request: &[u8],
    ip: &[u8; 4],
    ttl: Duration,
    buf: &mut [u8],
) -> Result<usize, DnsError> {
    let buf = Buf(buf, 0);

    let message = domain::base::Message::from_octets(request)?;
    debug!(
        "Processing message with header: {:?}",
        debug2format!(message.header())
    );

    let mut responseb = domain::base::MessageBuilder::from_target(buf)?;

    let buf = if matches!(message.header().opcode(), Opcode::QUERY) {
        debug!("Message is of type Query, processing all questions");

        let mut answerb = responseb.start_answer(&message, Rcode::NOERROR)?;

        for question in message.question() {
            let question = question?;

            if matches!(question.qtype(), Rtype::A) && matches!(question.qclass(), Class::IN) {
                let record = Record::new(
                    question.qname(),
                    Class::IN,
                    Ttl::from_duration_lossy(ttl),
                    A::from_octets(ip[0], ip[1], ip[2], ip[3]),
                );
                debug!(
                    "Answering {:?} with {:?}",
                    debug2format!(question),
                    debug2format!(record)
                );
                answerb.push(record)?;
            } else {
                debug!(
                    "Question {:?} is not of type A, not answering",
                    debug2format!(question)
                );
            }
        }

        answerb.finish()
    } else {
        debug!("Message is not of type Query, replying with NotImp");

        let headerb = responseb.header_mut();

        headerb.set_id(message.header().id());
        headerb.set_opcode(message.header().opcode());
        headerb.set_rd(message.header().rd());
        headerb.set_rcode(domain::base::iana::Rcode::NOTIMP);

        responseb.finish()
    };

    Ok(buf.1)
}

struct Buf<'a>(pub &'a mut [u8], pub usize);

impl Composer for Buf<'_> {}

impl OctetsBuilder for Buf<'_> {
    type AppendError = ShortBuf;

    fn append_slice(&mut self, slice: &[u8]) -> Result<(), Self::AppendError> {
        if self.1 + slice.len() <= self.0.len() {
            let end = self.1 + slice.len();
            self.0[self.1..end].copy_from_slice(slice);
            self.1 = end;

            Ok(())
        } else {
            Err(ShortBuf)
        }
    }
}

impl Truncate for Buf<'_> {
    fn truncate(&mut self, len: usize) {
        self.1 = len;
    }
}

impl AsMut<[u8]> for Buf<'_> {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.0[..self.1]
    }
}

impl AsRef<[u8]> for Buf<'_> {
    fn as_ref(&self) -> &[u8] {
        &self.0[..self.1]
    }
}
