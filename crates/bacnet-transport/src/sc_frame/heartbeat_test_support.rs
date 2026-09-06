//! Independent wire fixtures shared by both receive-path regression suites.

pub(crate) struct InvalidControl {
    pub name: &'static str,
    pub wire: Vec<u8>,
    pub nak: Option<Vec<u8>>,
    pub node_nak: Option<Vec<u8>>,
}

impl InvalidControl {
    fn without_node_reply(mut self) -> Self {
        self.node_nak = None;
        self
    }
}

fn case(
    name: &'static str,
    control: u8,
    fields: &[u8],
    destination: Option<[u8; 6]>,
    marker: u8,
    code: u8,
) -> InvalidControl {
    let mut wire = vec![0x0A, control, 0x22, 0x33];
    wire.extend_from_slice(fields);
    let mut nak = vec![0x00, if destination.is_some() { 4 } else { 0 }, 0x22, 0x33];
    if let Some(destination) = destination {
        nak.extend_from_slice(&destination);
    }
    nak.extend_from_slice(&[0x0A, 1, marker, 0, 7, 0, code]);
    InvalidControl {
        name,
        wire,
        node_nak: Some(nak.clone()),
        nak: Some(nak),
    }
}

pub(crate) fn invalid_heartbeats(local: [u8; 6]) -> Vec<InvalidControl> {
    let mut cases = vec![
        case("explicit source", 8, &[0x22; 6], Some([0x22; 6]), 0, 0x50),
        case("local destination", 4, &local, None, 0, 0x50).without_node_reply(),
        case("unrelated destination", 4, &[0x44; 6], None, 0, 0x50).without_node_reply(),
        case("Data Options", 1, &[0x1E], None, 0, 0x50),
        case("payload", 0, &[0x42], None, 0, 7),
        case("unsupported MU", 2, &[0x5E], None, 0x5E, 0x92),
        // A MU-clear predecessor, first MU with More Options + empty Header
        // Data, and another MU option. Decoding/re-encoding loses bit 5.
        case(
            "first raw MU marker",
            2,
            &[0x9E, 0xFE, 0, 0, 0x5D],
            None,
            0xFE,
            0x92,
        ),
        case("payload precedes MU", 2, &[0x5E, 0x42], None, 0, 7),
        case(
            "Data Options precede payload and MU",
            3,
            &[0x5E, 0x1E, 0x42],
            None,
            0,
            0x50,
        ),
    ];
    let mut fields = vec![0x22; 6];
    fields.extend_from_slice(&local);
    fields.extend_from_slice(&[0x5E, 0x1E, 0x42]);
    cases.push(
        case(
            "VMACs precede payload and MU",
            15,
            &fields,
            Some([0x22; 6]),
            0,
            0x50,
        )
        .without_node_reply(),
    );
    for source in [[0; 6], [0xFF; 6]] {
        for extra_faults in [false, true] {
            let mut fields = source.to_vec();
            if extra_faults {
                fields.extend_from_slice(&[0x5E, 0x42]);
            }
            let mut invalid = case(
                "reserved source is silent",
                if extra_faults { 10 } else { 8 },
                &fields,
                None,
                0,
                0x50,
            );
            invalid.nak = None;
            invalid.node_nak = None;
            cases.push(invalid);
        }
    }
    for source in [None, Some([0x22; 6])] {
        for extra_faults in [false, true] {
            let mut fields = source.map_or_else(Vec::new, |source| source.to_vec());
            fields.extend_from_slice(&[0xFF; 6]);
            if extra_faults {
                fields.extend_from_slice(&[0x5E, 0x42]);
            }
            let control =
                4 | if source.is_some() { 8 } else { 0 } | if extra_faults { 2 } else { 0 };
            let mut invalid = case(
                "broadcast destination is silent",
                control,
                &fields,
                None,
                0,
                0x50,
            );
            invalid.nak = None;
            invalid.node_nak = None;
            cases.push(invalid);
        }
    }
    cases
}

pub(crate) fn invalid_disconnects(local: [u8; 6]) -> Vec<InvalidControl> {
    let mut cases = invalid_heartbeats(local);
    // Literal recipient expectations for all source/destination combinations.
    for (source, destination, hub_reply) in [
        ([0x22; 6], local, true),
        ([0x22; 6], [0x44; 6], true),
        ([0x22; 6], [0xFF; 6], false),
        ([0; 6], local, false),
        ([0; 6], [0x44; 6], false),
        ([0; 6], [0xFF; 6], false),
        ([0xFF; 6], local, false),
        ([0xFF; 6], [0x44; 6], false),
        ([0xFF; 6], [0xFF; 6], false),
    ] {
        let mut fields = source.to_vec();
        fields.extend_from_slice(&destination);
        let mut invalid =
            case("source and destination", 12, &fields, Some(source), 0, 0x50).without_node_reply();
        if !hub_reply {
            invalid.nak = None;
        }
        cases.push(invalid);
    }
    // AB.2.12–15 use the same envelope. Only the Request function and
    // Result-For octet differ in these independent wire expectations.
    for case in &mut cases {
        case.wire[0] = 0x08;
        for nak in [&mut case.nak, &mut case.node_nak].into_iter().flatten() {
            let payload_start = nak.len() - 7;
            nak[payload_start] = 0x08;
        }
    }
    cases
}
