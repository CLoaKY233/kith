//! The answer engine: one query in, one decision (and usually one response) out.
//!
//! [`answer`] parses a query, decides whether the zone is authoritative, a split-DNS
//! route applies, or the default upstream should handle it, and for authoritative names
//! writes the complete response straight into the caller's buffer. It allocates nothing:
//! the query name is lowered into a stack buffer and looked up as a borrowed `&str`.

use crate::name::{DomainName, self_and_ancestors};
use crate::record::{Record, RecordData, RecordType};
use crate::split::{SplitDns, UpstreamGroupId};
use crate::wire::{Malformed, NameText, Overflow, Reader, Writer};
use crate::zone::Zone;

/// Longest CNAME chain followed inside the zone. Bounds the work per query and breaks
/// CNAME loops.
pub const MAX_CNAME_CHAIN: usize = 8;
/// UDP payload size assumed when the query has no EDNS OPT record (RFC 1035 §4.2.1).
pub const MIN_UDP_PAYLOAD: u16 = 512;
/// UDP payload size kith advertises in its own OPT record (the DNS Flag Day 2020 value).
pub const ADVERTISED_UDP_PAYLOAD: u16 = 1232;

const HEADER_LEN: usize = 12;
const QUESTION_OFFSET: u16 = 12;
const OPT_RECORD_LEN: usize = 11;
const CLASS_IN: u16 = 1;
const TYPE_OPT: u16 = 41;
const TYPE_IXFR: u16 = 251;
const TYPE_AXFR: u16 = 252;
const TYPE_ANY: u16 = 255;
const SOA_REFRESH: u32 = 3600;
const SOA_RETRY: u32 = 600;
const SOA_EXPIRE: u32 = 86_400;

/// Response codes the engine produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Rcode {
    /// No error.
    NoError,
    /// The query was malformed.
    FormErr,
    /// The name does not exist.
    NxDomain,
    /// The opcode is not supported.
    NotImp,
    /// The query is well-formed but will not be answered (class, zone transfer).
    Refused,
    /// The EDNS version is not supported (RFC 6891), carried partly in the OPT record.
    BadVers,
}

impl Rcode {
    /// The full 12-bit code.
    pub const fn code(self) -> u16 {
        match self {
            Self::NoError => 0,
            Self::FormErr => 1,
            Self::NxDomain => 3,
            Self::NotImp => 4,
            Self::Refused => 5,
            Self::BadVers => 16,
        }
    }

    /// The low four bits, carried in the header.
    fn header_bits(self) -> u8 {
        u8::try_from(self.code() % 16).unwrap_or(0)
    }

    /// The high eight bits, carried in the OPT record (RFC 6891 §6.1.3).
    fn extended_bits(self) -> u8 {
        u8::try_from(self.code() / 16).unwrap_or(0)
    }
}

/// What the caller should do with a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    /// An authoritative response of this many bytes is in `out` (NOERROR, NODATA or
    /// NXDOMAIN, possibly truncated with TC set).
    Answered(usize),
    /// An error response of `len` bytes is in `out`; send it back.
    Rejected {
        /// Response length.
        len: usize,
        /// Why the query was rejected.
        rcode: Rcode,
    },
    /// Forward the query unchanged to this upstream group.
    Forward {
        /// The group of the longest matching split-DNS route.
        upstream_group: UpstreamGroupId,
    },
    /// Forward the query unchanged to the default upstream.
    ForwardDefault,
}

/// Queries for which no response can be written. The caller drops them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DnsError {
    /// Shorter than a DNS header, so there is no ID to answer.
    #[error("query is shorter than a DNS header")]
    TooShort,
    /// The QR bit is set. Answering responses would enable reflection loops.
    #[error("message is a response, not a query")]
    NotAQuery,
    /// `out` cannot hold even the header and question.
    #[error("output buffer is too small for a response")]
    OutputTooSmall,
}

/// The transport a query arrived on, which sets the response size limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transport {
    Datagram,
    Stream,
}

/// Answers a query received over UDP. The response is limited to the client's EDNS UDP
/// size (512 without EDNS) and to `out.len()`; anything larger is truncated with TC.
pub fn answer(
    zone: &Zone,
    split: &SplitDns,
    query: &[u8],
    out: &mut [u8],
) -> Result<Outcome, DnsError> {
    respond(zone, split, query, out, Transport::Datagram)
}

/// Answers a query received over TCP (no length prefix). The response is limited only by
/// `out.len()` and the 65535-byte message maximum.
pub fn answer_stream(
    zone: &Zone,
    split: &SplitDns,
    query: &[u8],
    out: &mut [u8],
) -> Result<Outcome, DnsError> {
    respond(zone, split, query, out, Transport::Stream)
}

/// A parsed standard query.
struct Query {
    id: u16,
    recursion_desired: bool,
    question_end: usize,
    name: NameText,
    qtype: u16,
    qclass: u16,
    edns: Option<Edns>,
}

#[derive(Clone, Copy)]
struct Edns {
    udp_payload: u16,
    version: u8,
}

enum ParseFailure {
    Malformed,
    NotImplemented,
}

impl From<Malformed> for ParseFailure {
    fn from(_: Malformed) -> Self {
        Self::Malformed
    }
}

fn respond(
    zone: &Zone,
    split: &SplitDns,
    query: &[u8],
    out: &mut [u8],
    transport: Transport,
) -> Result<Outcome, DnsError> {
    let &[id_high, id_low, flags, ..] = query else {
        return Err(DnsError::TooShort);
    };
    if query.len() < HEADER_LEN {
        return Err(DnsError::TooShort);
    }
    let id = u16::from_be_bytes([id_high, id_low]);
    if flags & 0x80 != 0 {
        return Err(DnsError::NotAQuery);
    }

    let parsed = match parse(query, id, flags) {
        Ok(parsed) => parsed,
        Err(ParseFailure::Malformed) => return reject_bare(out, id, flags, Rcode::FormErr),
        Err(ParseFailure::NotImplemented) => return reject_bare(out, id, flags, Rcode::NotImp),
    };

    if parsed.qclass != CLASS_IN || matches!(parsed.qtype, TYPE_AXFR | TYPE_IXFR) {
        return reject_with_question(query, out, &parsed, Rcode::Refused);
    }
    if parsed.edns.is_some_and(|edns| edns.version != 0) {
        return reject_with_question(query, out, &parsed, Rcode::BadVers);
    }

    let decision = match route(zone, split, parsed.name.as_str()) {
        Route::Forward(upstream_group) => return Ok(Outcome::Forward { upstream_group }),
        Route::Default => return Ok(Outcome::ForwardDefault),
        authoritative @ (Route::Authoritative(_) | Route::Reverse(_)) => authoritative,
    };

    let limit = match transport {
        Transport::Datagram => usize::from(parsed.edns.map_or(MIN_UDP_PAYLOAD, |edns| {
            edns.udp_payload.max(MIN_UDP_PAYLOAD)
        })),
        Transport::Stream => usize::from(u16::MAX),
    };
    let opt_reserve = if parsed.edns.is_some() {
        OPT_RECORD_LEN
    } else {
        0
    };
    let section_limit = limit
        .checked_sub(opt_reserve)
        .ok_or(DnsError::OutputTooSmall)?;

    let mut writer = Writer::new(out, section_limit);
    start_response(&mut writer, query, &parsed, true).map_err(|_| DnsError::OutputTooSmall)?;
    let after_question = writer.position();

    let mut sections = Sections {
        answers: 0,
        authority: 0,
        rcode: Rcode::NoError,
    };
    let truncated =
        write_sections(&mut writer, zone, split, &parsed, &decision, &mut sections).is_err();
    if truncated {
        writer.truncate(after_question);
        sections.answers = 0;
        sections.authority = 0;
    }

    writer.set_limit(limit);
    let additional = finish_opt(&mut writer, parsed.edns, Rcode::NoError)
        .map_err(|_| DnsError::OutputTooSmall)?;
    let len = finish_header(
        &mut writer,
        sections.rcode,
        truncated,
        [sections.answers, sections.authority, additional],
    )
    .map_err(|_| DnsError::OutputTooSmall)?;
    Ok(Outcome::Answered(len))
}

fn parse(query: &[u8], id: u16, flags: u8) -> Result<Query, ParseFailure> {
    // Opcode is bits 3..=6 of the third header byte; only QUERY (0) is supported.
    if flags & 0x78 != 0 {
        return Err(ParseFailure::NotImplemented);
    }
    let mut reader = Reader::new(query, 4);
    let counts = [reader.u16()?, reader.u16()?, reader.u16()?, reader.u16()?];
    let [questions, answers, authority, additional] = counts;
    if questions != 1 || answers != 0 || authority != 0 {
        return Err(ParseFailure::Malformed);
    }
    let mut name = NameText::new();
    reader.question_name(&mut name)?;
    let qtype = reader.u16()?;
    let qclass = reader.u16()?;
    let question_end = reader.position();
    if qtype == TYPE_OPT {
        return Err(ParseFailure::Malformed);
    }

    let mut edns = None;
    for _ in 0..additional {
        let root = reader.skip_name()?;
        let rtype = reader.u16()?;
        let class = reader.u16()?;
        let ttl = reader.u32()?;
        let rdlength = reader.u16()?;
        reader.skip(usize::from(rdlength))?;
        if rtype == TYPE_OPT {
            if edns.is_some() || !root {
                return Err(ParseFailure::Malformed);
            }
            edns = Some(Edns {
                udp_payload: class,
                version: ttl.to_be_bytes()[1],
            });
        }
    }
    if !reader.is_empty() {
        return Err(ParseFailure::Malformed);
    }
    Ok(Query {
        id,
        recursion_desired: flags & 0x01 != 0,
        question_end,
        name,
        qtype,
        qclass,
        edns,
    })
}

/// Where a name is answered.
enum Route<'a> {
    /// Inside this authoritative apex.
    Authoritative(&'a str),
    /// A device address's reverse name, pointing at this name.
    Reverse(&'a DomainName),
    Forward(UpstreamGroupId),
    Default,
}

/// Device reverse names win; otherwise the longest matching apex or split-DNS route
/// wins, and at the same suffix the zone wins over a route.
fn route<'a>(zone: &'a Zone, split: &SplitDns, name: &'a str) -> Route<'a> {
    if let Some(target) = zone.reverse_target(name) {
        return Route::Reverse(target);
    }
    for suffix in self_and_ancestors(name) {
        if zone.is_apex(suffix) {
            return Route::Authoritative(suffix);
        }
        if let Some(group) = split.exact(suffix) {
            return Route::Forward(group);
        }
    }
    Route::Default
}

struct Sections {
    answers: u16,
    authority: u16,
    rcode: Rcode,
}

/// The owner of an answer record.
#[derive(Clone, Copy)]
enum Owner<'a> {
    /// The question name, written as a pointer so the client's spelling is echoed.
    Question,
    Name(&'a str),
}

enum Data<'a> {
    Record(&'a RecordData),
    Ptr(&'a DomainName),
    Soa {
        apex: &'a str,
        serial: u32,
        minimum: u32,
    },
}

impl Data<'_> {
    fn code(&self) -> u16 {
        match self {
            Self::Record(data) => data.record_type().code(),
            Self::Ptr(_) => RecordType::Ptr.code(),
            Self::Soa { .. } => RecordType::Soa.code(),
        }
    }
}

fn write_sections(
    writer: &mut Writer<'_>,
    zone: &Zone,
    split: &SplitDns,
    query: &Query,
    decision: &Route<'_>,
    sections: &mut Sections,
) -> Result<(), Overflow> {
    let qtype = query.qtype;
    let mut apex = match *decision {
        Route::Reverse(target) => {
            if qtype == RecordType::Ptr.code() || qtype == TYPE_ANY {
                write_record(
                    writer,
                    Owner::Question,
                    zone.device_ttl(),
                    &Data::Ptr(target),
                )?;
                sections.answers = sections.answers.saturating_add(1);
            }
            return Ok(());
        }
        Route::Authoritative(apex) => apex,
        Route::Forward(_) | Route::Default => return Ok(()),
    };
    let mut name = query.name.as_str();
    let mut owner = Owner::Question;

    for depth in 0..=MAX_CNAME_CHAIN {
        let Some(records) = zone.find(name, apex) else {
            sections.rcode = Rcode::NxDomain;
            return write_soa(writer, zone, apex, sections, false);
        };
        let alias = records.iter().find_map(|record| match record.data() {
            RecordData::Cname(target) => Some((record, target)),
            RecordData::A(_) | RecordData::Aaaa(_) | RecordData::Txt(_) | RecordData::Srv(_) => {
                None
            }
        });
        if let Some((record, target)) = alias
            && qtype != RecordType::Cname.code()
            && qtype != TYPE_ANY
        {
            write_answer(writer, owner, record, sections)?;
            if depth == MAX_CNAME_CHAIN {
                return Ok(());
            }
            match route(zone, split, target.as_str()) {
                Route::Authoritative(next_apex) => {
                    name = target.as_str();
                    apex = next_apex;
                    owner = Owner::Name(name);
                    continue;
                }
                // Out of zone: the client's resolver chases the rest.
                Route::Reverse(_) | Route::Forward(_) | Route::Default => return Ok(()),
            }
        }

        let at_apex = name == apex;
        let mut answered = false;
        if at_apex && (qtype == RecordType::Soa.code() || qtype == TYPE_ANY) {
            write_soa(writer, zone, apex, sections, true)?;
            answered = true;
        }
        for record in records
            .iter()
            .filter(|r| qtype == TYPE_ANY || r.record_type().code() == qtype)
        {
            write_answer(writer, owner, record, sections)?;
            answered = true;
        }
        if !answered {
            write_soa(writer, zone, apex, sections, false)?;
        }
        return Ok(());
    }
    Ok(())
}

fn write_answer(
    writer: &mut Writer<'_>,
    owner: Owner<'_>,
    record: &Record,
    sections: &mut Sections,
) -> Result<(), Overflow> {
    write_record(writer, owner, record.ttl(), &Data::Record(record.data()))?;
    sections.answers = sections.answers.checked_add(1).ok_or(Overflow)?;
    Ok(())
}

/// The SOA for `apex`, in the answer section or (for negative answers) the authority.
fn write_soa(
    writer: &mut Writer<'_>,
    zone: &Zone,
    apex: &str,
    sections: &mut Sections,
    in_answer: bool,
) -> Result<(), Overflow> {
    let minimum = zone.negative_ttl();
    let data = Data::Soa {
        apex,
        serial: zone.serial(),
        minimum,
    };
    write_record(writer, Owner::Name(apex), minimum, &data)?;
    let count = if in_answer {
        &mut sections.answers
    } else {
        &mut sections.authority
    };
    *count = count.checked_add(1).ok_or(Overflow)?;
    Ok(())
}

/// Writes one resource record, or nothing if it does not fit.
fn write_record(
    writer: &mut Writer<'_>,
    owner: Owner<'_>,
    ttl: u32,
    data: &Data<'_>,
) -> Result<(), Overflow> {
    let start = writer.position();
    let result = write_record_unchecked(writer, owner, ttl, data);
    if result.is_err() {
        writer.truncate(start);
    }
    result
}

fn write_record_unchecked(
    writer: &mut Writer<'_>,
    owner: Owner<'_>,
    ttl: u32,
    data: &Data<'_>,
) -> Result<(), Overflow> {
    match owner {
        Owner::Question => writer.pointer(QUESTION_OFFSET)?,
        Owner::Name(name) => writer.name(name)?,
    }
    writer.u16(data.code())?;
    writer.u16(CLASS_IN)?;
    writer.u32(ttl)?;
    let length_at = writer.position();
    writer.u16(0)?;
    let rdata_start = writer.position();
    match data {
        Data::Record(RecordData::A(address)) => writer.bytes(&address.octets())?,
        Data::Record(RecordData::Aaaa(address)) => writer.bytes(&address.octets())?,
        Data::Record(RecordData::Cname(target)) => writer.name(target.as_str())?,
        Data::Ptr(target) => writer.name(target.as_str())?,
        Data::Record(RecordData::Txt(txt)) => {
            for text in txt.strings() {
                writer.u8(u8::try_from(text.len()).map_err(|_| Overflow)?)?;
                writer.bytes(text.as_bytes())?;
            }
        }
        Data::Record(RecordData::Srv(srv)) => {
            writer.u16(srv.priority)?;
            writer.u16(srv.weight)?;
            writer.u16(srv.port)?;
            writer.name_uncompressed(srv.target.as_str())?;
        }
        Data::Soa {
            apex,
            serial,
            minimum,
        } => {
            writer.name(apex)?;
            writer.label("hostmaster")?;
            writer.name(apex)?;
            for value in [*serial, SOA_REFRESH, SOA_RETRY, SOA_EXPIRE, *minimum] {
                writer.u32(value)?;
            }
        }
    }
    let rdata_len = writer.position().checked_sub(rdata_start).ok_or(Overflow)?;
    writer.patch_u16(length_at, u16::try_from(rdata_len).map_err(|_| Overflow)?)
}

/// Header with zero counts, then (if `with_question`) the question copied verbatim.
fn start_response(
    writer: &mut Writer<'_>,
    query: &[u8],
    parsed: &Query,
    with_question: bool,
) -> Result<(), Overflow> {
    writer.u16(parsed.id)?;
    let flags = 0x80 | 0x04 | u8::from(parsed.recursion_desired);
    writer.bytes(&[flags, 0x80])?;
    writer.u16(u16::from(with_question))?;
    writer.bytes(&[0; 6])?;
    if with_question {
        let question = query.get(HEADER_LEN..parsed.question_end).ok_or(Overflow)?;
        writer.bytes(question)?;
        writer.remember_name_at(HEADER_LEN);
    }
    Ok(())
}

/// Writes the OPT record if the query had one, returning the additional count.
fn finish_opt(writer: &mut Writer<'_>, edns: Option<Edns>, rcode: Rcode) -> Result<u16, Overflow> {
    if edns.is_none() {
        return Ok(0);
    }
    writer.u8(0)?;
    writer.u16(TYPE_OPT)?;
    writer.u16(ADVERTISED_UDP_PAYLOAD)?;
    writer.bytes(&[rcode.extended_bits(), 0, 0, 0])?;
    writer.u16(0)?;
    Ok(1)
}

/// Fills in RCODE, TC and the section counts. Returns the message length.
fn finish_header(
    writer: &mut Writer<'_>,
    rcode: Rcode,
    truncated: bool,
    counts: [u16; 3],
) -> Result<usize, Overflow> {
    writer.patch_u8(3, |byte| (byte & 0xF0) | rcode.header_bits())?;
    if truncated {
        writer.patch_u8(2, |byte| byte | 0x02)?;
    }
    for (index, count) in counts.into_iter().enumerate() {
        let at = index
            .checked_mul(2)
            .and_then(|o| o.checked_add(6))
            .ok_or(Overflow)?;
        writer.patch_u16(at, count)?;
    }
    Ok(writer.position())
}

/// FORMERR or NOTIMP: just the header, echoing the ID, opcode and RD.
fn reject_bare(out: &mut [u8], id: u16, flags: u8, rcode: Rcode) -> Result<Outcome, DnsError> {
    let mut writer = Writer::new(out, HEADER_LEN);
    let header = (|| {
        writer.u16(id)?;
        writer.bytes(&[0x80 | (flags & 0x79), 0x80 | rcode.header_bits()])?;
        writer.bytes(&[0; 8])
    })();
    header.map_err(|_| DnsError::OutputTooSmall)?;
    Ok(Outcome::Rejected {
        len: writer.position(),
        rcode,
    })
}

/// REFUSED or BADVERS: header and question (and OPT when the query had EDNS).
fn reject_with_question(
    query: &[u8],
    out: &mut [u8],
    parsed: &Query,
    rcode: Rcode,
) -> Result<Outcome, DnsError> {
    let mut writer = Writer::new(out, usize::from(u16::MAX));
    let result = (|| {
        start_response(&mut writer, query, parsed, true)?;
        writer.patch_u8(2, |byte| byte & !0x04)?;
        let additional = finish_opt(&mut writer, parsed.edns, rcode)?;
        finish_header(&mut writer, rcode, false, [0, 0, additional])
    })();
    let len = result.map_err(|_| DnsError::OutputTooSmall)?;
    Ok(Outcome::Rejected { len, rcode })
}

#[cfg(test)]
mod tests;
