#![expect(
    clippy::arithmetic_side_effects,
    reason = "the test-only parser works on small, bounded test messages"
)]

use alloc::borrow::ToOwned;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use proptest::prelude::*;

use super::*;
use crate::record::{Srv, Txt};
use crate::zone::{DEFAULT_NEGATIVE_TTL, ZoneBuilder};

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

const LAPTOP_V4: Ipv4Addr = Ipv4Addr::new(100, 64, 0, 1);
const LAPTOP_V6: Ipv6Addr = Ipv6Addr::new(0xfd7a, 0x115c, 0xa1e0, 0, 0, 0, 0, 1);

fn name(text: &str) -> DomainName {
    DomainName::new(text).unwrap()
}

fn record(data: RecordData) -> Record {
    Record::new(300, data).unwrap()
}

fn zone() -> Zone {
    let mut builder = ZoneBuilder::new();
    builder.add_suffix(name("home.example.com")).unwrap();
    builder.add_suffix(name("net.kith.internal")).unwrap();
    builder.serial(7);
    builder
        .add_device("laptop", &[IpAddr::V4(LAPTOP_V4), IpAddr::V6(LAPTOP_V6)])
        .unwrap();
    builder
        .add_device("nas", &[IpAddr::V4(Ipv4Addr::new(100, 64, 0, 2))])
        .unwrap();
    let custom = [
        (
            "www.home.example.com",
            RecordData::Cname(name("web.home.example.com")),
        ),
        (
            "web.home.example.com",
            RecordData::Cname(name("nas.net.kith.internal")),
        ),
        (
            "ext.home.example.com",
            RecordData::Cname(name("example.org")),
        ),
        (
            "loop-a.home.example.com",
            RecordData::Cname(name("loop-b.home.example.com")),
        ),
        (
            "loop-b.home.example.com",
            RecordData::Cname(name("loop-a.home.example.com")),
        ),
        (
            "home.example.com",
            RecordData::Txt(Txt::new(vec!["v=spf1 -all".to_owned(), "second".to_owned()]).unwrap()),
        ),
        (
            "_http._tcp.home.example.com",
            RecordData::Srv(Srv {
                priority: 10,
                weight: 5,
                port: 8080,
                target: name("nas.home.example.com"),
            }),
        ),
        (
            "*.apps.home.example.com",
            RecordData::A(Ipv4Addr::new(100, 64, 0, 9)),
        ),
        (
            "real.apps.home.example.com",
            RecordData::A(Ipv4Addr::new(100, 64, 0, 10)),
        ),
        (
            "x.lab.apps.home.example.com",
            RecordData::A(Ipv4Addr::new(100, 64, 0, 11)),
        ),
    ];
    for (owner, data) in custom {
        builder.add_record(name(owner), record(data)).unwrap();
    }
    builder.build().unwrap()
}

fn split() -> SplitDns {
    let mut split = SplitDns::new();
    split
        .add_route(name("example.com"), UpstreamGroupId(1))
        .unwrap();
    split
        .add_route(name("corp.example.com"), UpstreamGroupId(2))
        .unwrap();
    // More specific than the zone suffix, so it wins inside it.
    split
        .add_route(name("vpn.home.example.com"), UpstreamGroupId(3))
        .unwrap();
    split
}

fn query(id: u16, qname: &str, qtype: u16, udp_payload: Option<u16>) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&id.to_be_bytes());
    bytes.extend_from_slice(&[
        0x01,
        0x00,
        0,
        1,
        0,
        0,
        0,
        0,
        0,
        u8::from(udp_payload.is_some()),
    ]);
    for label in qname.split('.').filter(|l| !l.is_empty()) {
        bytes.push(u8::try_from(label.len()).unwrap());
        bytes.extend_from_slice(label.as_bytes());
    }
    bytes.push(0);
    bytes.extend_from_slice(&qtype.to_be_bytes());
    bytes.extend_from_slice(&CLASS_IN.to_be_bytes());
    if let Some(size) = udp_payload {
        bytes.extend_from_slice(&[0, 0, 41]);
        bytes.extend_from_slice(&size.to_be_bytes());
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    }
    bytes
}

fn ask(qname: &str, qtype: RecordType) -> Message {
    let mut out = [0_u8; 4096];
    let bytes = query(0xBEEF, qname, qtype.code(), None);
    let Outcome::Answered(len) = answer(&zone(), &split(), &bytes, &mut out).unwrap() else {
        panic!("{qname} was not answered");
    };
    Message::parse(&out[..len]).unwrap()
}

// ---------------------------------------------------------------------------
// A strict, independent response parser. It rejects forward or looping pointers,
// compressed SRV targets, rdlength mismatches and trailing bytes.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Rd {
    A(Ipv4Addr),
    Aaaa(Ipv6Addr),
    Name(String),
    Txt(Vec<Vec<u8>>),
    Srv(u16, u16, u16, String),
    Soa {
        mname: String,
        rname: String,
        serial: u32,
        minimum: u32,
    },
    Opt {
        udp_payload: u16,
        extended_rcode: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Rr {
    name: String,
    rtype: u16,
    ttl: u32,
    data: Rd,
}

#[derive(Debug)]
struct Message {
    id: u16,
    flags: u16,
    question: Option<(String, u16)>,
    answers: Vec<Rr>,
    authority: Vec<Rr>,
    additional: Vec<Rr>,
}

impl Message {
    fn rcode(&self) -> u16 {
        self.flags & 0x0F
    }

    fn truncated(&self) -> bool {
        self.flags & 0x0200 != 0
    }

    fn authoritative(&self) -> bool {
        self.flags & 0x0400 != 0
    }

    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let mut at = 0;
        let u16_at = |at: usize| -> Result<u16, String> {
            Ok(u16::from_be_bytes(
                bytes.get(at..at + 2).ok_or("short")?.try_into().unwrap(),
            ))
        };
        let id = u16_at(0)?;
        let flags = u16_at(2)?;
        if flags & 0x8000 == 0 {
            return Err("QR not set".into());
        }
        let counts = [u16_at(4)?, u16_at(6)?, u16_at(8)?, u16_at(10)?];
        at += 12;
        let question = match counts[0] {
            0 => None,
            1 => {
                let qname = read_name(bytes, &mut at, true)?;
                let qtype = u16_at(at)?;
                at += 4;
                Some((qname, qtype))
            }
            _ => return Err("qdcount".into()),
        };
        let mut sections = [Vec::new(), Vec::new(), Vec::new()];
        for (section, &count) in sections.iter_mut().zip(&counts[1..]) {
            for _ in 0..count {
                section.push(read_rr(bytes, &mut at)?);
            }
        }
        if at != bytes.len() {
            return Err("trailing bytes".into());
        }
        let [answers, authority, additional] = sections;
        Ok(Self {
            id,
            flags,
            question,
            answers,
            authority,
            additional,
        })
    }
}

fn read_name(bytes: &[u8], at: &mut usize, allow_pointers: bool) -> Result<String, String> {
    let mut labels: Vec<String> = Vec::new();
    let mut cursor = *at;
    let mut jumped = false;
    let mut lowest_target = cursor;
    loop {
        let length = *bytes.get(cursor).ok_or("name runs off the end")?;
        if length & 0xC0 == 0xC0 {
            if !allow_pointers {
                return Err("compressed where forbidden".into());
            }
            let low = *bytes.get(cursor + 1).ok_or("short pointer")?;
            let target = usize::from(u16::from_be_bytes([length & 0x3F, low]));
            // Pointers must go strictly backwards, which rules out loops.
            if target >= lowest_target {
                return Err("forward or looping pointer".into());
            }
            lowest_target = target;
            if !jumped {
                *at = cursor + 2;
                jumped = true;
            }
            cursor = target;
            continue;
        }
        if length & 0xC0 != 0 {
            return Err("bad label type".into());
        }
        cursor += 1;
        if length == 0 {
            break;
        }
        let label = bytes
            .get(cursor..cursor + usize::from(length))
            .ok_or("short label")?;
        labels.push(String::from_utf8_lossy(label).to_ascii_lowercase());
        cursor += usize::from(length);
    }
    if !jumped {
        *at = cursor;
    }
    let text = labels.join(".");
    if text.len() > 253 {
        return Err("name too long".into());
    }
    Ok(text)
}

fn read_rr(bytes: &[u8], at: &mut usize) -> Result<Rr, String> {
    let name = read_name(bytes, at, true)?;
    let fixed = bytes.get(*at..*at + 10).ok_or("short rr")?;
    let rtype = u16::from_be_bytes([fixed[0], fixed[1]]);
    let class = u16::from_be_bytes([fixed[2], fixed[3]]);
    let ttl = u32::from_be_bytes([fixed[4], fixed[5], fixed[6], fixed[7]]);
    let rdlength = usize::from(u16::from_be_bytes([fixed[8], fixed[9]]));
    *at += 10;
    let end = *at + rdlength;
    let rdata = bytes.get(*at..end).ok_or("short rdata")?;
    if rtype != TYPE_OPT && class != CLASS_IN {
        return Err("class".into());
    }
    let mut cursor = *at;
    let data = match rtype {
        1 => {
            cursor = end;
            Rd::A(Ipv4Addr::from(
                <[u8; 4]>::try_from(rdata).map_err(|_| "A length")?,
            ))
        }
        28 => {
            cursor = end;
            Rd::Aaaa(Ipv6Addr::from(
                <[u8; 16]>::try_from(rdata).map_err(|_| "AAAA length")?,
            ))
        }
        5 | 12 => Rd::Name(read_name(bytes, &mut cursor, true)?),
        16 => {
            let mut strings = Vec::new();
            let mut rest = rdata;
            while let Some((&length, tail)) = rest.split_first() {
                let (text, tail) = tail
                    .split_at_checked(usize::from(length))
                    .ok_or("short txt")?;
                strings.push(text.to_vec());
                rest = tail;
            }
            cursor = end;
            Rd::Txt(strings)
        }
        33 => {
            let field = |i: usize| u16::from_be_bytes([rdata[i], rdata[i + 1]]);
            if rdata.len() < 7 {
                return Err("short srv".into());
            }
            cursor += 6;
            let target = read_name(bytes, &mut cursor, false)?;
            Rd::Srv(field(0), field(2), field(4), target)
        }
        6 => {
            let mname = read_name(bytes, &mut cursor, true)?;
            let rname = read_name(bytes, &mut cursor, true)?;
            let numbers = bytes.get(cursor..cursor + 20).ok_or("short soa")?;
            cursor += 20;
            let serial = u32::from_be_bytes([numbers[0], numbers[1], numbers[2], numbers[3]]);
            let minimum = u32::from_be_bytes([numbers[16], numbers[17], numbers[18], numbers[19]]);
            Rd::Soa {
                mname,
                rname,
                serial,
                minimum,
            }
        }
        41 => {
            cursor = end;
            Rd::Opt {
                udp_payload: class,
                extended_rcode: fixed[4],
            }
        }
        other => return Err(alloc::format!("unexpected type {other}")),
    };
    if cursor != end {
        return Err("rdlength mismatch".into());
    }
    *at = end;
    Ok(Rr {
        name,
        rtype,
        ttl,
        data,
    })
}

// ---------------------------------------------------------------------------
// Behaviour.
// ---------------------------------------------------------------------------

/// Changing these bytes is a protocol break.
#[test]
fn a_query_produces_golden_response_bytes() {
    let mut out = [0_u8; 512];
    let bytes = query(0x1234, "laptop.home.example.com", 1, None);
    let Outcome::Answered(len) = answer(&zone(), &split(), &bytes, &mut out).unwrap() else {
        panic!("not answered");
    };
    let mut expected = vec![0x12, 0x34, 0x85, 0x80, 0, 1, 0, 1, 0, 0, 0, 0];
    expected.extend_from_slice(b"\x06laptop\x04home\x07example\x03com\x00\x00\x01\x00\x01");
    expected.extend_from_slice(&[0xC0, 0x0C, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 100, 64, 0, 1]);
    assert_eq!(&out[..len], expected.as_slice());
}

#[test]
fn every_record_type_decodes_with_compressed_names() {
    let aaaa = ask("laptop.net.kith.internal", RecordType::Aaaa);
    assert_eq!(aaaa.answers.len(), 1);
    assert_eq!(aaaa.answers[0].data, Rd::Aaaa(LAPTOP_V6));
    assert_eq!(aaaa.answers[0].name, "laptop.net.kith.internal");

    let txt = ask("home.example.com", RecordType::Txt);
    assert_eq!(
        txt.answers[0].data,
        Rd::Txt(vec![b"v=spf1 -all".to_vec(), b"second".to_vec()])
    );

    let srv = ask("_http._tcp.home.example.com", RecordType::Srv);
    assert_eq!(
        srv.answers[0].data,
        Rd::Srv(10, 5, 8080, "nas.home.example.com".into())
    );

    // A two-link chain that crosses from one suffix into another.
    let chain = ask("www.home.example.com", RecordType::A);
    let got: Vec<(&str, &Rd)> = chain
        .answers
        .iter()
        .map(|rr| (rr.name.as_str(), &rr.data))
        .collect();
    assert_eq!(
        got,
        [
            (
                "www.home.example.com",
                &Rd::Name("web.home.example.com".into())
            ),
            (
                "web.home.example.com",
                &Rd::Name("nas.net.kith.internal".into())
            ),
            (
                "nas.net.kith.internal",
                &Rd::A(Ipv4Addr::new(100, 64, 0, 2))
            ),
        ]
    );
    assert_eq!(chain.rcode(), 0);
    assert!(chain.authoritative());
}

#[test]
fn cname_chain_stops_at_zone_edge_and_at_depth_limit() {
    let external = ask("ext.home.example.com", RecordType::A);
    assert_eq!(external.answers.len(), 1);
    assert_eq!(external.answers[0].data, Rd::Name("example.org".into()));

    let looped = ask("loop-a.home.example.com", RecordType::A);
    assert_eq!(looped.answers.len(), MAX_CNAME_CHAIN + 1);
    assert_eq!(looped.rcode(), 0);

    // Asking for the CNAME itself does not follow it.
    let direct = ask("www.home.example.com", RecordType::Cname);
    assert_eq!(direct.answers.len(), 1);
}

#[test]
fn missing_names_are_nxdomain_and_missing_types_are_nodata_both_with_soa() {
    let missing = ask("nope.home.example.com", RecordType::A);
    assert_eq!(missing.rcode(), 3);
    assert_eq!(missing.answers, []);
    assert_eq!(missing.authority.len(), 1);
    assert_eq!(
        missing.authority[0].data,
        Rd::Soa {
            mname: "home.example.com".into(),
            rname: "hostmaster.home.example.com".into(),
            serial: 7,
            minimum: DEFAULT_NEGATIVE_TTL,
        }
    );

    for (owner, qtype) in [
        ("laptop.home.example.com", RecordType::Txt),
        // The apex and empty non-terminals exist too.
        ("net.kith.internal", RecordType::A),
        ("lab.apps.home.example.com", RecordType::A),
    ] {
        let nodata = ask(owner, qtype);
        assert_eq!(nodata.rcode(), 0, "{owner}");
        assert_eq!(nodata.answers, [], "{owner}");
        assert_eq!(nodata.authority[0].rtype, RecordType::Soa.code(), "{owner}");
    }

    let soa = ask("net.kith.internal", RecordType::Soa);
    assert_eq!(soa.answers.len(), 1);
}

#[test]
fn wildcard_answers_only_names_below_its_closest_encloser() {
    let synthesized = ask("anything.apps.home.example.com", RecordType::A);
    assert_eq!(
        synthesized.answers[0].name,
        "anything.apps.home.example.com"
    );
    assert_eq!(
        synthesized.answers[0].data,
        Rd::A(Ipv4Addr::new(100, 64, 0, 9))
    );
    assert_eq!(
        ask("deep.er.apps.home.example.com", RecordType::A)
            .answers
            .len(),
        1
    );

    // An existing name is never covered.
    let real = ask("real.apps.home.example.com", RecordType::A);
    assert_eq!(real.answers[0].data, Rd::A(Ipv4Addr::new(100, 64, 0, 10)));
    // `lab` exists as an empty non-terminal, so it is the closest encloser and has no
    // wildcard of its own.
    assert_eq!(ask("y.lab.apps.home.example.com", RecordType::A).rcode(), 3);
}

#[test]
fn matching_ignores_case_and_echoes_the_clients_spelling() {
    let mut out = [0_u8; 512];
    let bytes = query(1, "LaPtOp.HOME.example.COM", 1, None);
    let Outcome::Answered(len) = answer(&zone(), &split(), &bytes, &mut out).unwrap() else {
        panic!("not answered");
    };
    assert!(out[..len].windows(6).any(|w| w == b"LaPtOp"));
    let message = Message::parse(&out[..len]).unwrap();
    assert_eq!(message.answers[0].data, Rd::A(LAPTOP_V4));
}

#[test]
fn device_addresses_get_ptr_records_to_the_primary_suffix() {
    let v4 = ask("1.0.64.100.in-addr.arpa", RecordType::Ptr);
    assert_eq!(
        v4.answers[0].data,
        Rd::Name("laptop.home.example.com".into())
    );
    // Written out independently of the implementation.
    let v6 = ask(
        "1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.e.1.a.c.5.1.1.a.7.d.f.ip6.arpa",
        RecordType::Ptr,
    );
    assert_eq!(
        v6.answers[0].data,
        Rd::Name("laptop.home.example.com".into())
    );

    let mut out = [0_u8; 512];
    let other = query(1, "9.9.64.100.in-addr.arpa", 12, None);
    assert_eq!(
        answer(&zone(), &split(), &other, &mut out),
        Ok(Outcome::ForwardDefault)
    );
}

#[test]
fn longest_suffix_wins_between_zone_and_split_routes() {
    let mut out = [0_u8; 512];
    let mut decide =
        |qname: &str| answer(&zone(), &split(), &query(1, qname, 1, None), &mut out).unwrap();
    assert_eq!(
        decide("www.example.com"),
        Outcome::Forward {
            upstream_group: UpstreamGroupId(1)
        }
    );
    assert_eq!(
        decide("a.corp.example.com"),
        Outcome::Forward {
            upstream_group: UpstreamGroupId(2)
        }
    );
    assert_eq!(
        decide("x.vpn.home.example.com"),
        Outcome::Forward {
            upstream_group: UpstreamGroupId(3)
        }
    );
    // The zone suffix is longer than the `example.com` route.
    assert!(matches!(
        decide("nope.home.example.com"),
        Outcome::Answered(_)
    ));
    assert_eq!(decide("rust-lang.org"), Outcome::ForwardDefault);
    // A label boundary matters: `badexample.com` is not under `example.com`.
    assert_eq!(decide("badexample.com"), Outcome::ForwardDefault);
}

#[test]
fn oversized_answers_set_tc_and_edns_raises_the_limit() {
    let mut builder = ZoneBuilder::new();
    builder.add_suffix(name("big.test")).unwrap();
    for i in 0..40 {
        builder
            .add_record(
                name("many.big.test"),
                record(RecordData::A(Ipv4Addr::new(10, 0, 0, i))),
            )
            .unwrap();
    }
    let zone = builder.build().unwrap();
    // 40 × 16 bytes of answers plus header and question exceed 512.
    let mut out = [0_u8; 2048];
    let Outcome::Answered(len) = answer(
        &zone,
        &split(),
        &query(1, "many.big.test", 1, None),
        &mut out,
    )
    .unwrap() else {
        panic!();
    };
    let small = Message::parse(&out[..len]).unwrap();
    assert!(small.truncated());
    assert_eq!(small.answers, []);
    assert!(small.question.is_some());

    let Outcome::Answered(len) = answer(
        &zone,
        &split(),
        &query(1, "many.big.test", 1, Some(4096)),
        &mut out,
    )
    .unwrap() else {
        panic!();
    };
    let large = Message::parse(&out[..len]).unwrap();
    assert!(!large.truncated());
    assert_eq!(large.answers.len(), 40);
    assert_eq!(
        large.additional[0].data,
        Rd::Opt {
            udp_payload: ADVERTISED_UDP_PAYLOAD,
            extended_rcode: 0
        }
    );

    // A caller buffer smaller than the client's limit truncates too, and keeps the OPT.
    let Outcome::Answered(len) = answer(
        &zone,
        &split(),
        &query(1, "many.big.test", 1, Some(4096)),
        &mut out[..100],
    )
    .unwrap() else {
        panic!();
    };
    let tiny = Message::parse(&out[..len]).unwrap();
    assert!(tiny.truncated());
    assert_eq!(tiny.additional.len(), 1);

    let Outcome::Answered(len) = answer_stream(
        &zone,
        &split(),
        &query(1, "many.big.test", 1, None),
        &mut out,
    )
    .unwrap() else {
        panic!();
    };
    assert_eq!(Message::parse(&out[..len]).unwrap().answers.len(), 40);
}

#[test]
fn malformed_and_unsupported_queries_get_the_right_error_or_are_dropped() {
    let zone = zone();
    let split = split();
    let mut out = [0_u8; 512];
    let mut run = |bytes: &[u8]| answer(&zone, &split, bytes, &mut out);

    assert_eq!(run(&[0; 11]), Err(DnsError::TooShort));
    let mut response = query(1, "laptop.home.example.com", 1, None);
    response[2] |= 0x80;
    assert_eq!(run(&response), Err(DnsError::NotAQuery));

    let formerr = Outcome::Rejected {
        len: 12,
        rcode: Rcode::FormErr,
    };
    // A header followed by garbage.
    assert_eq!(
        run(&[0xAB, 0xCD, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF]),
        Ok(formerr)
    );
    let mut two_questions = query(1, "a.home.example.com", 1, None);
    two_questions[5] = 2;
    assert_eq!(run(&two_questions), Ok(formerr));
    let mut trailing = query(1, "a.home.example.com", 1, None);
    trailing.push(0);
    assert_eq!(run(&trailing), Ok(formerr));

    let mut notify = query(1, "a.home.example.com", 1, None);
    notify[2] = 0x20;
    assert_eq!(
        run(&notify),
        Ok(Outcome::Rejected {
            len: 12,
            rcode: Rcode::NotImp
        })
    );

    let axfr = query(1, "home.example.com", 252, None);
    assert!(matches!(
        run(&axfr),
        Ok(Outcome::Rejected {
            rcode: Rcode::Refused,
            ..
        })
    ));

    let mut version_one = query(1, "home.example.com", 1, Some(1232));
    let opt_version = version_one.len() - 5;
    version_one[opt_version] = 1;
    let Ok(Outcome::Rejected {
        len,
        rcode: Rcode::BadVers,
    }) = run(&version_one)
    else {
        panic!()
    };
    let message = Message::parse(&out[..len]).unwrap();
    assert_eq!(
        message.additional[0].data,
        Rd::Opt {
            udp_payload: ADVERTISED_UDP_PAYLOAD,
            extended_rcode: 1
        }
    );
    assert_eq!(message.id, 1);
}

// ---------------------------------------------------------------------------
// Properties.
// ---------------------------------------------------------------------------

/// Arbitrary bytes, plus well-formed queries with random bytes flipped, which reach
/// deeper than pure noise does.
fn query_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        proptest::collection::vec(any::<u8>(), 0..600),
        (
            proptest::collection::vec((any::<usize>(), any::<u8>()), 0..4),
            any::<bool>()
        )
            .prop_map(|(flips, edns)| {
                let mut bytes = query(9, "www.home.example.com", 1, edns.then_some(700));
                for (index, value) in flips {
                    let len = bytes.len();
                    bytes[index % len] = value;
                }
                bytes
            }),
    ]
}

proptest! {
    #[test]
    fn arbitrary_input_never_panics_or_writes_past_the_buffer(
        bytes in query_bytes(),
        size in 0_usize..1500,
    ) {
        let zone = zone();
        let mut out = vec![0xA5_u8; 1600];
        let result = answer(&zone, &split(), &bytes, &mut out[..size]);
        prop_assert!(out[size..].iter().all(|&b| b == 0xA5));
        if let Ok(Outcome::Answered(len) | Outcome::Rejected { len, .. }) = result {
            prop_assert!(len <= size);
            prop_assert!(Message::parse(&out[..len]).is_ok());
        }
    }

    #[test]
    fn every_response_from_a_random_zone_reparses_and_matches_the_zone(
        devices in proptest::collection::btree_map("[a-c]{1,3}", any::<u32>(), 0..6),
        records in proptest::collection::vec(("[a-c*]{1,2}", 0_u8..4, "[a-c]{1,3}", any::<u32>()), 0..12),
        qname in "[a-c*]{1,3}(\\.[a-c]{1,3})?\\.z\\.test",
        qtype in prop_oneof![Just(1_u16), Just(5), Just(6), Just(12), Just(16), Just(28), Just(33), Just(255), any::<u16>()],
        udp in proptest::option::of(0_u16..2000),
        size in 0_usize..2048,
    ) {
        let mut builder = ZoneBuilder::new();
        builder.add_suffix(name("z.test")).unwrap();
        for (label, address) in &devices {
            builder.add_device(label, &[IpAddr::V4(Ipv4Addr::from(*address))]).unwrap();
        }
        for (owner, kind, target, value) in &records {
            let Ok(owner) = DomainName::new(&alloc::format!("{owner}.z.test")) else { continue };
            let target = name(&alloc::format!("{target}.z.test"));
            let data = match kind {
                0 => RecordData::A(Ipv4Addr::from(*value)),
                1 => RecordData::Cname(target),
                2 => RecordData::Txt(Txt::new(vec![value.to_string()]).unwrap()),
                _ => RecordData::Srv(Srv { priority: 1, weight: 2, port: 3, target }),
            };
            builder.add_record(owner, record(data)).unwrap();
        }
        // Many random zones are invalid (CNAME conflicts, device clashes); that is fine.
        let Ok(zone) = builder.build() else { return Ok(()) };
        let mut out = vec![0_u8; size];
        let bytes = query(77, &qname, qtype, udp);
        match answer(&zone, &SplitDns::new(), &bytes, &mut out) {
            Ok(Outcome::Answered(len)) => {
                let message = Message::parse(&out[..len]).map_err(TestCaseError::fail)?;
                prop_assert_eq!(message.id, 77);
                prop_assert!(message.authoritative());
                prop_assert_eq!(&message.question, &Some((qname.clone(), qtype)));
                prop_assert!(len <= usize::from(udp.map_or(512, |u| u.max(512))));
                // The first answer, if any, is for the question name.
                if let Some(first) = message.answers.first() {
                    prop_assert_eq!(&first.name, &qname);
                }
                // A bare A answer must match what the zone stores (or its wildcard).
                if qtype == 1 && !message.truncated() {
                    for rr in message.answers.iter().filter(|rr| rr.rtype == 1) {
                        let stored = zone.find(&rr.name, "z.test").unwrap();
                        let Rd::A(address) = rr.data else { continue };
                        prop_assert!(stored.iter().any(|r| *r.data() == RecordData::A(address)));
                    }
                }
            }
            Ok(Outcome::Rejected { len, .. }) => {
                prop_assert!(Message::parse(&out[..len]).is_ok());
            }
            Ok(other) => prop_assert!(false, "unexpected {other:?}"),
            Err(error) => prop_assert_eq!(error, DnsError::OutputTooSmall),
        }
    }
}
