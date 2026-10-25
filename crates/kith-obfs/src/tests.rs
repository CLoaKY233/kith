use alloc::vec;
use alloc::vec::Vec;

use proptest::prelude::*;

use crate::{
    EncodeError, Inbound, MAX_JUNK_COUNT, MessageType, ObfsParams, ParamsError, RandomSource,
    RawObfsParams,
};

/// Deterministic source: emits `next, next + 1, ...` (wrapping).
struct Counter(u8);

impl RandomSource for Counter {
    fn fill(&mut self, buf: &mut [u8]) {
        for byte in buf {
            *byte = self.0;
            self.0 = self.0.wrapping_add(1);
        }
    }
}

/// Deterministic source replaying a byte vector, then zeros.
struct Bytes(Vec<u8>, usize);

impl RandomSource for Bytes {
    fn fill(&mut self, buf: &mut [u8]) {
        for byte in buf {
            *byte = self.0.get(self.1).copied().unwrap_or(0);
            self.1 = self.1.wrapping_add(1);
        }
    }
}

fn sample_raw() -> RawObfsParams {
    RawObfsParams {
        junk_count: 3,
        junk_min: 10,
        junk_max: 20,
        init_padding: 5,
        response_padding: 7,
        header_init: 0x1111_1111,
        header_response: 0x2222_2222,
        header_cookie: 0x3333_3333,
        header_transport: 0x4444_4444,
    }
}

fn sample() -> ObfsParams {
    ObfsParams::new(sample_raw()).unwrap()
}

fn wg_message(kind: MessageType, len: usize) -> Vec<u8> {
    let mut message: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
    message[..4].copy_from_slice(&kind.wireguard_header().to_le_bytes());
    message
}

const KINDS: [(MessageType, usize); 5] = [
    (MessageType::Initiation, 148),
    (MessageType::Response, 92),
    (MessageType::CookieReply, 64),
    (MessageType::Transport, 32),
    (MessageType::Transport, 1420),
];

fn round_trip(params: &ObfsParams, kind: MessageType, len: usize, rng: &mut impl RandomSource) {
    let message = wg_message(kind, len);
    let mut out = vec![0_u8; 2048];
    let written = params.encode(&message, &mut out, rng).unwrap();
    assert_eq!(written, len + params.padding_len(kind));
    let padding = params.padding_len(kind);
    assert_eq!(
        out[padding..padding + 4],
        params.header(kind).to_le_bytes(),
        "header remapped"
    );
    match params.decode(&mut out[..written]) {
        Inbound::Wireguard {
            kind: got,
            message: restored,
        } => {
            assert_eq!(got, kind);
            assert_eq!(restored, &message[..]);
        }
        other => panic!("expected {kind:?}, got {other:?}"),
    }
}

#[test]
fn round_trip_all_message_types() {
    for (kind, len) in KINDS {
        round_trip(&sample(), kind, len, &mut Counter(0));
    }
}

#[test]
fn in_place_transport_round_trip() {
    let params = sample();
    let original = wg_message(MessageType::Transport, 100);
    let mut packet = original.clone();
    assert_eq!(
        params.encode_in_place(&mut packet),
        Ok(MessageType::Transport)
    );
    assert_eq!(packet[..4], 0x4444_4444_u32.to_le_bytes());
    assert_eq!(packet[4..], original[4..]);
    match params.decode(&mut packet) {
        Inbound::Wireguard { kind, message } => {
            assert_eq!(kind, MessageType::Transport);
            assert_eq!(message, &original[..]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn in_place_rejects_padded_handshake() {
    let mut init = wg_message(MessageType::Initiation, 148);
    assert_eq!(
        sample().encode_in_place(&mut init),
        Err(EncodeError::NeedsPadding)
    );
}

#[test]
fn encode_rejects_non_wireguard_and_small_buffers() {
    let params = sample();
    let mut out = [0_u8; 256];
    let bad_type = wg_message(MessageType::Transport, 40)
        .into_iter()
        .enumerate()
        .map(|(i, b)| if i == 0 { 9 } else { b })
        .collect::<Vec<_>>();
    assert_eq!(
        params.encode(&bad_type, &mut out, &mut Counter(0)),
        Err(EncodeError::NotWireguard)
    );
    let bad_size = wg_message(MessageType::Initiation, 147);
    assert_eq!(
        params.encode(&bad_size, &mut out, &mut Counter(0)),
        Err(EncodeError::NotWireguard)
    );
    let mut reserved = wg_message(MessageType::Transport, 40);
    reserved[1] = 1;
    assert_eq!(
        params.encode(&reserved, &mut out, &mut Counter(0)),
        Err(EncodeError::NotWireguard)
    );
    let init = wg_message(MessageType::Initiation, 148);
    assert_eq!(
        params.encode(&init, &mut out[..152], &mut Counter(0)),
        Err(EncodeError::BufferTooSmall { needed: 153 })
    );
    assert_eq!(
        params.junk_packet(&mut out[..19], &mut Counter(0)),
        Err(EncodeError::BufferTooSmall { needed: 20 })
    );
}

#[test]
fn identity_is_a_no_op() {
    let params = ObfsParams::identity();
    assert!(params.is_identity());
    assert_eq!(ObfsParams::default(), params);
    assert_eq!(ObfsParams::new(RawObfsParams::default()), Ok(params));
    for (kind, len) in KINDS {
        let message = wg_message(kind, len);
        let mut out = vec![0_u8; 2048];
        let written = params.encode(&message, &mut out, &mut Counter(0)).unwrap();
        assert_eq!(&out[..written], &message[..]);
        let mut in_place = message.clone();
        assert_eq!(params.encode_in_place(&mut in_place), Ok(kind));
        assert_eq!(in_place, message);
        round_trip(&params, kind, len, &mut Counter(0));
    }
    assert!(!sample().is_identity());
    // No junk with identity params: short garbage is unrecognized.
    assert_eq!(params.decode(&mut [0_u8; 0]), Inbound::Unrecognized);
}

#[test]
fn junk_is_classified_as_junk() {
    let params = sample();
    let mut rng = Counter(0);
    let mut lengths = Vec::new();
    for _ in 0..params.junk_count() {
        let mut out = [0_u8; 20];
        let len = params.junk_packet(&mut out, &mut rng).unwrap();
        assert!((10..=20).contains(&len));
        lengths.push(len);
        assert_eq!(params.decode(&mut out[..len]), Inbound::Junk);
    }
    // Sizes outside [Jmin, Jmax] that match nothing are unrecognized.
    assert_eq!(params.decode(&mut [0_u8; 9]), Inbound::Unrecognized);
    assert_eq!(params.decode(&mut [0_u8; 21]), Inbound::Unrecognized);
}

#[test]
fn wrong_header_is_not_accepted() {
    let params = sample();
    // A correctly sized initiation with the plain WG header is not an AWG one.
    let mut padded = vec![0_u8; 153];
    padded[5..9].copy_from_slice(&1_u32.to_le_bytes());
    assert_eq!(params.decode(&mut padded), Inbound::Unrecognized);
    // A 64-byte message with H4 is transport data, not a cookie.
    let mut cookie_sized = vec![0_u8; 64];
    cookie_sized[..4].copy_from_slice(&0x4444_4444_u32.to_le_bytes());
    assert!(matches!(
        params.decode(&mut cookie_sized),
        Inbound::Wireguard {
            kind: MessageType::Transport,
            ..
        }
    ));
}

#[test]
fn validation_accepts_bounds() {
    let mut raw = sample_raw();
    raw.junk_count = MAX_JUNK_COUNT;
    raw.junk_min = 65_534;
    raw.junk_max = 65_534;
    raw.init_padding = 65_535 - 149;
    raw.response_padding = 65_535 - 94;
    assert!(ObfsParams::new(raw).is_ok());
}

#[test]
fn validation_rules() {
    let check = |edit: fn(&mut RawObfsParams), expected: ParamsError| {
        let mut raw = sample_raw();
        edit(&mut raw);
        assert_eq!(ObfsParams::new(raw), Err(expected));
    };
    check(
        |r| r.junk_count = 129,
        ParamsError::JunkCountTooLarge { count: 129 },
    );
    check(
        |r| r.junk_min = 21,
        ParamsError::JunkRangeInverted { min: 21, max: 20 },
    );
    check(
        |r| {
            r.junk_min = 0;
            r.junk_max = 65_535;
        },
        ParamsError::JunkTooLarge { max: 65_535 },
    );
    check(
        |r| r.init_padding = 65_535 - 148,
        ParamsError::InitPaddingTooLarge { padding: 65_387 },
    );
    check(
        |r| r.response_padding = 65_535 - 92,
        ParamsError::ResponsePaddingTooLarge { padding: 65_443 },
    );
    check(
        |r| {
            r.init_padding = 0;
            r.response_padding = 56;
        },
        ParamsError::PaddedSizesCollide { size: 148 },
    );
    check(
        |r| r.header_response = r.header_init,
        ParamsError::DuplicateHeaders,
    );
    check(
        |r| r.header_cookie = r.header_init,
        ParamsError::DuplicateHeaders,
    );
    check(
        |r| r.header_transport = r.header_init,
        ParamsError::DuplicateHeaders,
    );
    check(
        |r| r.header_cookie = r.header_response,
        ParamsError::DuplicateHeaders,
    );
    check(
        |r| r.header_transport = r.header_response,
        ParamsError::DuplicateHeaders,
    );
    check(
        |r| r.header_transport = r.header_cookie,
        ParamsError::DuplicateHeaders,
    );
    // Jmin/Jmax are validated even with Jc = 0.
    check(
        |r| {
            r.junk_count = 0;
            r.junk_min = 30;
        },
        ParamsError::JunkRangeInverted { min: 30, max: 20 },
    );
}

#[test]
fn deserialize_validates() {
    let good = postcard::to_allocvec(&sample()).unwrap();
    assert_eq!(postcard::from_bytes::<ObfsParams>(&good).unwrap(), sample());
    let mut raw = sample_raw();
    raw.header_cookie = raw.header_init;
    let bad = postcard::to_allocvec(&raw).unwrap();
    assert!(postcard::from_bytes::<ObfsParams>(&bad).is_err());
}

#[test]
fn golden_initiation() {
    let message = wg_message(MessageType::Initiation, 148);
    let mut out = [0_u8; 153];
    let written = sample()
        .encode(&message, &mut out, &mut Counter(0xA0))
        .unwrap();
    assert_eq!(written, 153);
    let mut expected = vec![0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0x11, 0x11, 0x11, 0x11];
    expected.extend_from_slice(&message[4..]);
    assert_eq!(out[..], expected[..]);
}

#[test]
fn golden_response_and_transport() {
    let params = sample();
    let response = wg_message(MessageType::Response, 92);
    let mut out = [0_u8; 99];
    params.encode(&response, &mut out, &mut Counter(0)).unwrap();
    assert_eq!(out[..11], [0, 1, 2, 3, 4, 5, 6, 0x22, 0x22, 0x22, 0x22]);
    let transport = wg_message(MessageType::Transport, 32);
    let mut out = [0_u8; 32];
    params
        .encode(&transport, &mut out, &mut Counter(0))
        .unwrap();
    assert_eq!(out[..6], [0x44, 0x44, 0x44, 0x44, 4, 5]);
}

#[test]
fn golden_junk() {
    // Length draw: u64 LE from bytes 0..8 = 0x0706050403020100; mod 11 = 6; 10 + 6 = 16.
    let mut out = [0_u8; 20];
    let len = sample().junk_packet(&mut out, &mut Counter(0)).unwrap();
    assert_eq!(len, 16);
    let expected: Vec<u8> = (8..24).collect();
    assert_eq!(out[..len], expected[..]);
}

#[test]
fn golden_random_params() {
    let params = ObfsParams::random(&mut Counter(0));
    assert_eq!(
        params.raw(),
        RawObfsParams {
            junk_count: GOLDEN_RANDOM.0,
            junk_min: GOLDEN_RANDOM.1,
            junk_max: GOLDEN_RANDOM.2,
            init_padding: GOLDEN_RANDOM.3,
            response_padding: GOLDEN_RANDOM.4,
            header_init: GOLDEN_RANDOM.5[0],
            header_response: GOLDEN_RANDOM.5[1],
            header_cookie: GOLDEN_RANDOM.5[2],
            header_transport: GOLDEN_RANDOM.5[3],
        }
    );
}

const GOLDEN_RANDOM: (u8, u16, u16, u16, u16, [u32; 4]) = (
    11,
    54,
    647,
    31,
    95,
    [386_992_910, 1_195_457_342, 2_003_921_774, 2_812_386_206],
);

#[test]
fn random_terminates_with_degenerate_sources() {
    for byte in [0_u8, 0xFF] {
        let params = ObfsParams::random(&mut Bytes(vec![byte; 1024], 0));
        assert!(ObfsParams::new(params.raw()).is_ok());
        assert!(!params.is_identity());
    }
}

proptest! {
    #[test]
    fn decode_never_panics(
        datagram in proptest::collection::vec(any::<u8>(), 0..2100),
        seed in proptest::collection::vec(any::<u8>(), 64),
    ) {
        let mut data = datagram.clone();
        let _ = ObfsParams::random(&mut Bytes(seed, 0)).decode(&mut data);
        let mut data = datagram.clone();
        let _ = sample().decode(&mut data);
        let mut data = datagram;
        let _ = ObfsParams::identity().decode(&mut data);
    }

    #[test]
    fn random_params_validate_and_round_trip(
        seed in proptest::collection::vec(any::<u8>(), 64),
        pad in proptest::collection::vec(any::<u8>(), 512),
        transport_len in 32_usize..1500,
    ) {
        let params = ObfsParams::random(&mut Bytes(seed, 0));
        prop_assert_eq!(ObfsParams::new(params.raw()), Ok(params));
        let mut rng = Bytes(pad, 0);
        for (kind, len) in KINDS.into_iter().chain([(MessageType::Transport, transport_len)]) {
            round_trip(&params, kind, len, &mut rng);
        }
        let mut out = vec![0_u8; usize::from(params.junk_max())];
        let len = params.junk_packet(&mut out, &mut rng).unwrap();
        let junk = matches!(params.decode(&mut out[..len]), Inbound::Junk | Inbound::Wireguard { .. });
        prop_assert!(junk);
    }

    #[test]
    fn arbitrary_raw_params_never_panic(
        junk_count: u8, junk_min: u16, junk_max: u16, init_padding: u16, response_padding: u16,
        h: [u32; 4],
    ) {
        let raw = RawObfsParams {
            junk_count, junk_min, junk_max, init_padding, response_padding,
            header_init: h[0], header_response: h[1], header_cookie: h[2], header_transport: h[3],
        };
        if let Ok(params) = ObfsParams::new(raw) {
            prop_assert_eq!(params.raw(), raw);
        }
    }
}
