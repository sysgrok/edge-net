#![cfg_attr(not(feature = "std"), no_std)]
#![warn(clippy::large_futures)]
#![allow(async_fn_in_trait)]
#![allow(clippy::uninlined_format_args)]
#![allow(unknown_lints)]

use core::cmp::Ordering;
use core::fmt::Display;
use core::ops::RangeBounds;

use domain::base::header::Flags;
use domain::base::iana::{Opcode, Rcode};
use domain::base::message::ShortMessage;
use domain::base::message_builder::PushError;
use domain::base::name::{FromStrError, Label, ToLabelIter};
use domain::base::rdata::ComposeRecordData;
use domain::base::wire::{Composer, ParseError};
use domain::base::{
    Message, MessageBuilder, ParsedName, Question, Record, RecordData, Rtype, ToName,
};
use domain::dep::octseq::{FreezeBuilder, FromBuilder, Octets, OctetsBuilder, ShortBuf, Truncate};
use domain::rdata::AllRecordData;

// This mod MUST go first, so that the others see its macros.
pub(crate) mod fmt;

#[cfg(feature = "io")]
pub mod buf; // TODO: Maybe move to a generic `edge-buf` crate in future
/// Re-export the domain lib if the user would like to directly
/// assemble / parse mDNS messages.
pub mod domain {
    pub use domain::*;
}
pub mod host;
#[cfg(feature = "io")]
pub mod io;

/// The DNS-SD owner name.
pub const DNS_SD_OWNER: NameSlice = NameSlice::new(&["_services", "_dns-sd", "_udp", "local"]);

/// A wrapper type for the errors returned by the `domain` library during parsing and
/// constructing mDNS messages.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum MdnsError {
    ShortBuf,
    InvalidMessage,
}

impl Display for MdnsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShortBuf => write!(f, "ShortBuf"),
            Self::InvalidMessage => write!(f, "InvalidMessage"),
        }
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for MdnsError {
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            Self::ShortBuf => defmt::write!(f, "ShortBuf"),
            Self::InvalidMessage => defmt::write!(f, "InvalidMessage"),
        }
    }
}

impl core::error::Error for MdnsError {}

impl From<ShortBuf> for MdnsError {
    fn from(_: ShortBuf) -> Self {
        Self::ShortBuf
    }
}

impl From<PushError> for MdnsError {
    fn from(_: PushError) -> Self {
        Self::ShortBuf
    }
}

impl From<FromStrError> for MdnsError {
    fn from(_: FromStrError) -> Self {
        Self::InvalidMessage
    }
}

impl From<ShortMessage> for MdnsError {
    fn from(_: ShortMessage) -> Self {
        Self::InvalidMessage
    }
}

impl From<ParseError> for MdnsError {
    fn from(_: ParseError) -> Self {
        Self::InvalidMessage
    }
}

/// This newtype struct allows the construction of a `domain` lib Name from
/// a bunch of `&str` labels represented as a slice.
///
/// Implements the `domain` lib `ToName` trait.
#[derive(Debug, Clone)]
pub struct NameSlice<'a>(&'a [&'a str]);

impl<'a> NameSlice<'a> {
    /// Create a new `NameSlice` instance from a slice of `&str` labels.
    pub const fn new(labels: &'a [&'a str]) -> Self {
        Self(labels)
    }
}

impl core::fmt::Display for NameSlice<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for label in self.0 {
            write!(f, "{}.", label)?;
        }

        Ok(())
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for NameSlice<'_> {
    fn format(&self, f: defmt::Formatter<'_>) {
        for label in self.0 {
            defmt::write!(f, "{}.", label);
        }
    }
}

impl ToName for NameSlice<'_> {}

/// An iterator over the labels in a `NameSlice` instance.
#[derive(Clone)]
pub struct NameSliceIter<'a> {
    name: &'a NameSlice<'a>,
    index: usize,
}

impl<'a> Iterator for NameSliceIter<'a> {
    type Item = &'a Label;

    fn next(&mut self) -> Option<Self::Item> {
        match self.index.cmp(&self.name.0.len()) {
            Ordering::Less => {
                let label = unwrap!(
                    Label::from_slice(self.name.0[self.index].as_bytes()),
                    "Unreachable"
                );
                self.index += 1;
                Some(label)
            }
            Ordering::Equal => {
                let label = Label::root();
                self.index += 1;
                Some(label)
            }
            Ordering::Greater => None,
        }
    }
}

impl DoubleEndedIterator for NameSliceIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.index > 0 {
            self.index -= 1;
            if self.index == self.name.0.len() {
                let label = Label::root();
                Some(label)
            } else {
                let label = unwrap!(
                    Label::from_slice(self.name.0[self.index].as_bytes()),
                    "Unreachable"
                );
                Some(label)
            }
        } else {
            None
        }
    }
}

impl ToLabelIter for NameSlice<'_> {
    type LabelIter<'t>
        = NameSliceIter<'t>
    where
        Self: 't;

    fn iter_labels(&self) -> Self::LabelIter<'_> {
        NameSliceIter {
            name: self,
            index: 0,
        }
    }
}

/// A custom struct for representing a TXT data record off from a slice of
/// key-value `&str` pairs.
#[derive(Debug, Clone)]
pub struct Txt<'a>(&'a [(&'a str, &'a str)]);

impl<'a> Txt<'a> {
    pub const fn new(txt: &'a [(&'a str, &'a str)]) -> Self {
        Self(txt)
    }
}

impl core::fmt::Display for Txt<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Txt [")?;

        for (i, (k, v)) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, ", {}={}", k, v)?;
            } else {
                write!(f, "{}={}", k, v)?;
            }
        }

        write!(f, "]")?;

        Ok(())
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for Txt<'_> {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "Txt [");

        for (i, (k, v)) in self.0.iter().enumerate() {
            if i > 0 {
                defmt::write!(f, ", {}={}", k, v);
            } else {
                defmt::write!(f, "{}={}", k, v);
            }
        }

        defmt::write!(f, "]");
    }
}

impl RecordData for Txt<'_> {
    fn rtype(&self) -> Rtype {
        Rtype::TXT
    }
}

impl ComposeRecordData for Txt<'_> {
    fn rdlen(&self, _compress: bool) -> Option<u16> {
        None
    }

    fn compose_rdata<Target: Composer + ?Sized>(
        &self,
        target: &mut Target,
    ) -> Result<(), Target::AppendError> {
        if self.0.is_empty() {
            target.append_slice(&[0])?;
        } else {
            // TODO: Will not work for (k, v) pairs larger than 254 bytes in length
            for (k, v) in self.0 {
                target.append_slice(&[(k.len() + v.len() + 1) as u8])?;
                target.append_slice(k.as_bytes())?;
                target.append_slice(b"=")?;
                target.append_slice(v.as_bytes())?;
            }
        }

        Ok(())
    }

    fn compose_canonical_rdata<Target: Composer + ?Sized>(
        &self,
        target: &mut Target,
    ) -> Result<(), Target::AppendError> {
        self.compose_rdata(target)
    }
}

/// A custom struct allowing to chain together multiple custom record data types.
/// Allows e.g. using the custom `Txt` struct from above and chain it with `domain`'s `AllRecordData`,
#[derive(Debug, Clone)]
pub enum RecordDataChain<T, U> {
    This(T),
    Next(U),
}

impl<T, U> core::fmt::Display for RecordDataChain<T, U>
where
    T: core::fmt::Display,
    U: core::fmt::Display,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::This(data) => write!(f, "{}", data),
            Self::Next(data) => write!(f, "{}", data),
        }
    }
}

#[cfg(feature = "defmt")]
impl<T, U> defmt::Format for RecordDataChain<T, U>
where
    T: defmt::Format,
    U: defmt::Format,
{
    fn format(&self, f: defmt::Formatter<'_>) {
        match self {
            Self::This(data) => defmt::write!(f, "{}", data),
            Self::Next(data) => defmt::write!(f, "{}", data),
        }
    }
}

impl<T, U> RecordData for RecordDataChain<T, U>
where
    T: RecordData,
    U: RecordData,
{
    fn rtype(&self) -> Rtype {
        match self {
            Self::This(data) => data.rtype(),
            Self::Next(data) => data.rtype(),
        }
    }
}

impl<T, U> ComposeRecordData for RecordDataChain<T, U>
where
    T: ComposeRecordData,
    U: ComposeRecordData,
{
    fn rdlen(&self, compress: bool) -> Option<u16> {
        match self {
            Self::This(data) => data.rdlen(compress),
            Self::Next(data) => data.rdlen(compress),
        }
    }

    fn compose_rdata<Target: Composer + ?Sized>(
        &self,
        target: &mut Target,
    ) -> Result<(), Target::AppendError> {
        match self {
            Self::This(data) => data.compose_rdata(target),
            Self::Next(data) => data.compose_rdata(target),
        }
    }

    fn compose_canonical_rdata<Target: Composer + ?Sized>(
        &self,
        target: &mut Target,
    ) -> Result<(), Target::AppendError> {
        match self {
            Self::This(data) => data.compose_canonical_rdata(target),
            Self::Next(data) => data.compose_canonical_rdata(target),
        }
    }
}

/// This struct allows one to use a regular `&mut [u8]` slice as an octet buffer
/// with the `domain` library.
///
/// Useful when a `domain` message needs to be constructed in a `&mut [u8]` slice.
pub struct Buf<'a>(pub &'a mut [u8], pub usize);

impl<'a> Buf<'a> {
    /// Create a new `Buf` instance from a mutable slice.
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self(buf, 0)
    }
}

impl FreezeBuilder for Buf<'_> {
    type Octets = Self;

    fn freeze(self) -> Self {
        self
    }
}

impl Octets for Buf<'_> {
    type Range<'r>
        = &'r [u8]
    where
        Self: 'r;

    fn range(&self, range: impl RangeBounds<usize>) -> Self::Range<'_> {
        self.0[..self.1].range(range)
    }
}

impl<'a> FromBuilder for Buf<'a> {
    type Builder = Buf<'a>;

    fn from_builder(builder: Self::Builder) -> Self {
        Buf(&mut builder.0[builder.1..], 0)
    }
}

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

/// Type of request for `MdnsHandler::handle`.
#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MdnsRequest<'a> {
    /// No incoming mDNS request. Send a broadcast message
    None,
    /// Incoming mDNS request
    Request {
        /// Whether it is a legacy request (i.e. UDP packet source port is not 5353, as per spec)
        legacy: bool,
        /// Whether the request arrived on the multicast address
        multicast: bool,
        /// The data of the request
        data: &'a [u8],
    },
}

/// Return type for `MdnsHandler::handle`.
#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MdnsResponse<'a> {
    None,
    Reply { data: &'a [u8], delay: bool },
}

/// A trait that abstracts the processing logic for an incoming mDNS message.
///
/// Handles an incoming mDNS message by parsing it and potentially preparing a response.
///
/// If request is `None`, the handler should prepare a broadcast message with
/// all its data (i.e. mDNS responder brodcasts on internal state changes).
///
/// Returns an `MdnsResponse` instance that instructs the caller
/// what data to send as a response (if any) and whether to generate a random delay
/// before sending (as per spec).
pub trait MdnsHandler {
    fn handle<'a>(
        &mut self,
        request: MdnsRequest<'_>,
        response_buf: &'a mut [u8],
    ) -> Result<MdnsResponse<'a>, MdnsError>;
}

impl<T> MdnsHandler for &mut T
where
    T: MdnsHandler,
{
    fn handle<'a>(
        &mut self,
        request: MdnsRequest<'_>,
        response_buf: &'a mut [u8],
    ) -> Result<MdnsResponse<'a>, MdnsError> {
        (**self).handle(request, response_buf)
    }
}

/// A structure representing a handler that does not do any processing.
///
/// Useful only when chaining multiple `MdnsHandler` instances.
pub struct NoHandler;

impl NoHandler {
    /// Chains a `NoHandler` with another handler.
    pub fn chain<T>(self, handler: T) -> ChainedHandler<T, Self> {
        ChainedHandler::new(handler, self)
    }
}

impl MdnsHandler for NoHandler {
    fn handle<'a>(
        &mut self,
        _request: MdnsRequest<'_>,
        _response_buf: &'a mut [u8],
    ) -> Result<MdnsResponse<'a>, MdnsError> {
        Ok(MdnsResponse::None)
    }
}

/// A composite handler that chains two handlers together.
pub struct ChainedHandler<T, U> {
    first: T,
    second: U,
}

impl<T, U> ChainedHandler<T, U> {
    /// Create a new `ChainedHandler` instance from two handlers.
    pub const fn new(first: T, second: U) -> Self {
        Self { first, second }
    }

    /// Chains a `ChainedHandler` with another handler,
    /// where our instance would be the first one to be called.
    ///
    /// Chaining works by calling each chained handler from the first to the last,
    /// until a handler in the chain returns a non-zero `usize` result.
    ///
    /// Once that happens, traversing the handlers down the chain stops.
    pub fn chain<V>(self, handler: V) -> ChainedHandler<V, Self> {
        ChainedHandler::new(handler, self)
    }
}

impl<T, U> MdnsHandler for ChainedHandler<T, U>
where
    T: MdnsHandler,
    U: MdnsHandler,
{
    fn handle<'a>(
        &mut self,
        request: MdnsRequest<'_>,
        response_buf: &'a mut [u8],
    ) -> Result<MdnsResponse<'a>, MdnsError> {
        match self.first.handle(request.clone(), response_buf)? {
            MdnsResponse::None => self.second.handle(request, response_buf),
            MdnsResponse::Reply { data, delay } => {
                let len = data.len();

                Ok(MdnsResponse::Reply {
                    data: &response_buf[..len],
                    delay,
                })
            }
        }
    }
}

/// A type alias for the answer which is expected to be returned by instances
/// implementing the `HostAnswers` trait.
pub type HostAnswer<'a> =
    Record<NameSlice<'a>, RecordDataChain<Txt<'a>, AllRecordData<&'a [u8], NameSlice<'a>>>>;

/// A trait that abstracts the logic for providing answers to incoming mDNS queries.
///
/// The visitor-pattern-with-a-callback is chosen on purpose, as that allows `domain`
/// Names to be constructed on-the-fly, possibly without interim buffer allocations.
///
/// Look at the implementation of `HostAnswers` for `host::Host` and `host::Service`
/// for examples of this technique.
pub trait HostAnswers {
    /// Visits an entity that does have answers to mDNS queries.
    ///
    /// The answers will be provided to the supplied `f` callback.
    ///
    /// Note that the entity should provide ALL of its answers, regardless of the
    /// concrete questions.
    ///
    /// The filtering of the answers relevant for the asked questions is done by the caller,
    /// and only if necessary (i.e. only if these answers are used to reply to a concrete mDNS query,
    /// rather than just broadcasting all answers that the entity has, which is also a valid mDNS
    /// operation, that should be done when the entity providing answers has changes ot its internal state).
    fn visit<F, E>(&self, f: F) -> Result<(), E>
    where
        F: FnMut(HostAnswer) -> Result<(), E>,
        E: From<MdnsError>;
}

impl<T> HostAnswers for &T
where
    T: HostAnswers,
{
    fn visit<F, E>(&self, f: F) -> Result<(), E>
    where
        F: FnMut(HostAnswer) -> Result<(), E>,
        E: From<MdnsError>,
    {
        (*self).visit(f)
    }
}

impl<T> HostAnswers for &mut T
where
    T: HostAnswers,
{
    fn visit<F, E>(&self, f: F) -> Result<(), E>
    where
        F: FnMut(HostAnswer) -> Result<(), E>,
        E: From<MdnsError>,
    {
        (**self).visit(f)
    }
}

/// A type alias for the question which is expected to be returned by instances
/// implementing the `HostQuestions` trait.
pub type HostQuestion<'a> = Question<NameSlice<'a>>;

/// A trait that abstracts the logic for providing questions to outgoing mDNS queries.
///
/// The visitor-pattern-with-a-callback is chosen on purpose, as that allows `domain`
/// Names to be constructed on-the-fly, possibly without interim buffer allocations.
pub trait HostQuestions {
    /// Visits an entity that does have questions.
    ///
    /// The questions will be provided to the supplied `f` callback.
    fn visit<F, E>(&self, f: F) -> Result<(), E>
    where
        F: FnMut(HostQuestion) -> Result<(), E>,
        E: From<MdnsError>;

    /// A function that constructs an mDNS query message in a `&mut [u8]` buffer
    /// using questions generated by this trait.
    fn query(&self, id: u16, buf: &mut [u8]) -> Result<usize, MdnsError> {
        let buf = Buf(buf, 0);

        let mut mb = MessageBuilder::from_target(buf)?;

        set_header(&mut mb, id, false);

        let mut qb = mb.question();

        let mut pushed = false;

        self.visit(|question| {
            qb.push(question)?;

            pushed = true;

            Ok::<_, MdnsError>(())
        })?;

        let buf = qb.finish();

        if pushed {
            Ok(buf.1)
        } else {
            Ok(0)
        }
    }
}

impl<T> HostQuestions for &T
where
    T: HostQuestions,
{
    fn visit<F, E>(&self, f: F) -> Result<(), E>
    where
        F: FnMut(HostQuestion) -> Result<(), E>,
        E: From<MdnsError>,
    {
        (*self).visit(f)
    }
}

impl<T> HostQuestions for &mut T
where
    T: HostQuestions,
{
    fn visit<F, E>(&self, f: F) -> Result<(), E>
    where
        F: FnMut(HostQuestion) -> Result<(), E>,
        E: From<MdnsError>,
    {
        (**self).visit(f)
    }
}

/// A structure modeling an entity that does not generate any questions.
///
/// Useful only when chaining multiple `HostQuestions` instances.
pub struct NoHostQuestions;

impl NoHostQuestions {
    /// Chains a `HostQuestions` with another `HostAnswers` instance.
    pub fn chain<T>(self, questions: T) -> ChainedHostQuestions<T, Self> {
        ChainedHostQuestions::new(questions, self)
    }
}

impl HostQuestions for NoHostQuestions {
    fn visit<F, E>(&self, _f: F) -> Result<(), E>
    where
        F: FnMut(HostQuestion) -> Result<(), E>,
    {
        Ok(())
    }
}

/// A composite `HostQuestions` that chains two `HostQuestions` instances together.
pub struct ChainedHostQuestions<T, U> {
    first: T,
    second: U,
}

impl<T, U> ChainedHostQuestions<T, U> {
    /// Create a new `ChainedHostQuestions` instance from two `HostQuestions` instances.
    pub const fn new(first: T, second: U) -> Self {
        Self { first, second }
    }

    /// Chains this instance with another `HostQuestions` instance,
    pub fn chain<V>(self, answers: V) -> ChainedHostQuestions<V, Self> {
        ChainedHostQuestions::new(answers, self)
    }
}

impl<T, U> HostQuestions for ChainedHostQuestions<T, U>
where
    T: HostQuestions,
    U: HostQuestions,
{
    fn visit<F, E>(&self, mut f: F) -> Result<(), E>
    where
        F: FnMut(HostQuestion) -> Result<(), E>,
        E: From<MdnsError>,
    {
        self.first.visit(&mut f)?;
        self.second.visit(f)
    }
}

/// A structure modeling an entity that does not generate any answers.
///
/// Useful only when chaining multiple `HostAnswers` instances.
pub struct NoHostAnswers;

impl NoHostAnswers {
    /// Chains a `NoHostAnswers` with another `HostAnswers` instance.
    pub fn chain<T>(self, answers: T) -> ChainedHostAnswers<T, Self> {
        ChainedHostAnswers::new(answers, self)
    }
}

impl HostAnswers for NoHostAnswers {
    fn visit<F, E>(&self, _f: F) -> Result<(), E>
    where
        F: FnMut(HostAnswer) -> Result<(), E>,
    {
        Ok(())
    }
}

/// A composite `HostAnswers` that chains two `HostAnswers` instances together.
pub struct ChainedHostAnswers<T, U> {
    first: T,
    second: U,
}

impl<T, U> ChainedHostAnswers<T, U> {
    /// Create a new `ChainedHostAnswers` instance from two `HostAnswers` instances.
    pub const fn new(first: T, second: U) -> Self {
        Self { first, second }
    }

    /// Chains this instance with another `HostAnswers` instance,
    pub fn chain<V>(self, answers: V) -> ChainedHostAnswers<V, Self> {
        ChainedHostAnswers::new(answers, self)
    }
}

impl<T, U> HostAnswers for ChainedHostAnswers<T, U>
where
    T: HostAnswers,
    U: HostAnswers,
{
    fn visit<F, E>(&self, mut f: F) -> Result<(), E>
    where
        F: FnMut(HostAnswer) -> Result<(), E>,
        E: From<MdnsError>,
    {
        self.first.visit(&mut f)?;
        self.second.visit(f)
    }
}

/// An `MdnsHandler` implementation that answers mDNS queries with the answers
/// provided by an entity implementing the `HostAnswers` trait.
///
/// Typically, this structure will be used to provide answers to other peers that broadcast
/// mDNS queries - i.e. this is the "responder" aspect of the mDNS protocol.
pub struct HostAnswersMdnsHandler<T> {
    answers: T,
}

impl<T> HostAnswersMdnsHandler<T> {
    /// Create a new `HostAnswersMdnsHandler` instance from an entity that provides answers.
    pub const fn new(answers: T) -> Self {
        Self { answers }
    }
}

impl<T> MdnsHandler for HostAnswersMdnsHandler<T>
where
    T: HostAnswers,
{
    fn handle<'a>(
        &mut self,
        request: MdnsRequest<'_>,
        response_buf: &'a mut [u8],
    ) -> Result<MdnsResponse<'a>, MdnsError> {
        let buf = Buf(response_buf, 0);

        let mut mb = MessageBuilder::from_target(buf)?;

        let mut pushed = false;

        let buf = if let MdnsRequest::Request { legacy, data, .. } = request {
            let message = Message::from_octets(data)?;

            if !matches!(message.header().opcode(), Opcode::QUERY)
                || !matches!(message.header().rcode(), Rcode::NOERROR)
                || message.header().qr()
            // Not a query but a response
            {
                return Ok(MdnsResponse::None);
            }

            let mut ab = if legacy {
                set_header(&mut mb, message.header().id(), true);

                let mut qb = mb.question();

                // As per spec, for legacy requests we need to fill-in the questions section
                for question in message.question() {
                    qb.push(question?)?;
                }

                qb.answer()
            } else {
                set_header(&mut mb, 0, true);

                mb.answer()
            };

            let mut additional_a = false;
            let mut additional_srv_txt = false;

            for question in message.question() {
                let question = question?;

                self.answers.visit(|answer| {
                    if matches!(answer.data(), RecordDataChain::Next(AllRecordData::Srv(_))) {
                        additional_a = true;
                    }

                    if !answer.owner().name_eq(&DNS_SD_OWNER)
                        && matches!(answer.data(), RecordDataChain::Next(AllRecordData::Ptr(_)))
                    {
                        additional_a = true;

                        // Over-simplifying here in that we'll send all our SRV and TXT records, however
                        // sending only some SRV and PTR records is too complex to implement.
                        additional_srv_txt = true;
                    }

                    if question.qname().name_eq(&answer.owner()) {
                        debug!(
                            "Answering question [{}] with: [{}]",
                            debug2format!(question),
                            debug2format!(answer)
                        );

                        ab.push(answer)?;

                        pushed = true;
                    }

                    Ok::<_, MdnsError>(())
                })?;
            }

            if additional_a || additional_srv_txt {
                // Fill-in the additional section as well

                let mut aa = ab.additional();

                self.answers.visit(|answer| {
                    if matches!(
                        answer.data(),
                        RecordDataChain::Next(AllRecordData::A(_))
                            | RecordDataChain::Next(AllRecordData::Aaaa(_))
                            | RecordDataChain::Next(AllRecordData::Srv(_))
                            | RecordDataChain::Next(AllRecordData::Txt(_))
                            | RecordDataChain::This(Txt(_))
                    ) {
                        debug!("Additional answer: [{}]", debug2format!(answer));

                        aa.push(answer)?;
                    }

                    Ok::<_, MdnsError>(())
                })?;

                aa.finish()
            } else {
                ab.finish()
            }
        } else {
            set_header(&mut mb, 0, true);

            let mut ab = mb.answer();

            self.answers.visit(|answer| {
                ab.push(answer)?;

                pushed = true;

                Ok::<_, MdnsError>(())
            })?;

            ab.finish()
        };

        if pushed {
            Ok(MdnsResponse::Reply {
                data: &buf.0[..buf.1],
                delay: false,
            })
        } else {
            Ok(MdnsResponse::None)
        }
    }
}

/// A type alias for the answer which is expected to be returned by instances
/// implementing the `PeerAnswers` trait.
pub type PeerAnswer<'a> =
    Record<ParsedName<&'a [u8]>, AllRecordData<&'a [u8], ParsedName<&'a [u8]>>>;

/// A trait that abstracts the logic for processing answers from incoming mDNS queries.
///
/// Rather than dealing with the whole mDNS message, this trait is focused on processing
/// the answers from the message (in the `answer` and `additional` mDNS message sections).
pub trait PeerAnswers {
    /// Processes the answers from an incoming mDNS message.
    fn answers<'a, T, A>(&self, answers: T, additional: A) -> Result<(), MdnsError>
    where
        T: IntoIterator<Item = Result<PeerAnswer<'a>, MdnsError>> + Clone + 'a,
        A: IntoIterator<Item = Result<PeerAnswer<'a>, MdnsError>> + Clone + 'a;
}

impl<T> PeerAnswers for &mut T
where
    T: PeerAnswers,
{
    fn answers<'a, U, V>(&self, answers: U, additional: V) -> Result<(), MdnsError>
    where
        U: IntoIterator<Item = Result<PeerAnswer<'a>, MdnsError>> + Clone + 'a,
        V: IntoIterator<Item = Result<PeerAnswer<'a>, MdnsError>> + Clone + 'a,
    {
        (**self).answers(answers, additional)
    }
}

impl<T> PeerAnswers for &T
where
    T: PeerAnswers,
{
    fn answers<'a, U, V>(&self, answers: U, additional: V) -> Result<(), MdnsError>
    where
        U: IntoIterator<Item = Result<PeerAnswer<'a>, MdnsError>> + Clone + 'a,
        V: IntoIterator<Item = Result<PeerAnswer<'a>, MdnsError>> + Clone + 'a,
    {
        (*self).answers(answers, additional)
    }
}

/// A structure implementing the `MdnsHandler` trait by processing all answers from an
/// incoming mDNS message via delegating to an entity implementing the `PeerAnswers` trait.
///
/// Typically, this structure will be used to process answers which are replies to mDNS
/// queries that we have sent using the `HostQuestions::query` method, i.e., this is the
/// "querying" part of the mDNS protocol.
///
/// Since the "querying" aspect of the mDNS protocol is modeled here, this handler never
/// answers anything, i.e. it always returns a 0 `usize`, because - unlike the
/// `HostAnswersMdnsHandler` - it does not have any answers to provide, as it - itself -
/// processes answers provided by peers, which were themselves sent because we issued a query
/// using e.g. the `HostQuestions::query` method at an earlier point in time.
pub struct PeerAnswersMdnsHandler<T> {
    answers: T,
}

impl<T> PeerAnswersMdnsHandler<T> {
    /// Create a new `PeerAnswersMdnsHandler` instance from an entity that processes answers.
    pub const fn new(answers: T) -> Self {
        Self { answers }
    }
}

impl<T> MdnsHandler for PeerAnswersMdnsHandler<T>
where
    T: PeerAnswers,
{
    fn handle<'a>(
        &mut self,
        request: MdnsRequest<'_>,
        _response_buf: &'a mut [u8],
    ) -> Result<MdnsResponse<'a>, MdnsError> {
        let MdnsRequest::Request { data, legacy, .. } = request else {
            return Ok(MdnsResponse::None);
        };

        if legacy {
            // Legacy packets should not contain mDNS answers anyway, per spec
            return Ok(MdnsResponse::None);
        }

        let message = Message::from_octets(data)?;

        if !matches!(message.header().opcode(), Opcode::QUERY)
            || !matches!(message.header().rcode(), Rcode::NOERROR)
            || !message.header().qr()
        // Not a response but a query
        {
            return Ok(MdnsResponse::None);
        }

        let answers = message.answer()?;
        let additional = message.additional()?;

        let answers = answers.filter_map(|answer| {
            match answer {
                Ok(answer) => answer.into_record::<AllRecordData<_, _>>(),
                Err(e) => Err(e),
            }
            .map_err(|_| MdnsError::InvalidMessage)
            .transpose()
        });

        let additional = additional.filter_map(|answer| {
            match answer {
                Ok(answer) => answer.into_record::<AllRecordData<_, _>>(),
                Err(e) => Err(e),
            }
            .map_err(|_| MdnsError::InvalidMessage)
            .transpose()
        });

        self.answers.answers(answers, additional)?;

        Ok(MdnsResponse::None)
    }
}

/// Utility function that sets the header of an mDNS `domain` message builder
/// to be a response or a query.
pub fn set_header<T: Composer>(answer: &mut MessageBuilder<T>, id: u16, response: bool) {
    let header = answer.header_mut();
    header.set_id(id);
    header.set_opcode(Opcode::QUERY);
    header.set_rcode(Rcode::NOERROR);

    let mut flags = Flags::new();
    flags.qr = response;
    flags.aa = response;
    header.set_flags(flags);
}
