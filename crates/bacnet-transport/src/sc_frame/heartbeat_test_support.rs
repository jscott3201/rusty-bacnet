//! Independent wire fixtures shared by both receive-path regression suites.

pub(crate) struct InvalidHeartbeat {
    pub name: &'static str,
    pub wire: Vec<u8>,
    pub nak: Option<Vec<u8>>,
}

fn case(
    name: &'static str,
    control: u8,
    fields: &[u8],
    destination: Option<[u8; 6]>,
    marker: u8,
    code: u8,
) -> InvalidHeartbeat {
    let mut wire = vec![0x0A, control, 0x22, 0x33];
    wire.extend_from_slice(fields);
    let mut nak = vec![0x00, if destination.is_some() { 4 } else { 0 }, 0x22, 0x33];
    if let Some(destination) = destination {
        nak.extend_from_slice(&destination);
    }
    nak.extend_from_slice(&[0x0A, 1, marker, 0, 7, 0, code]);
    InvalidHeartbeat {
        name,
        wire,
        nak: Some(nak),
    }
}

pub(crate) fn invalid_heartbeats(local: [u8; 6]) -> Vec<InvalidHeartbeat> {
    let mut cases = vec![
        case("explicit source", 8, &[0x22; 6], Some([0x22; 6]), 0, 0x50),
        case("local destination", 4, &local, None, 0, 0x50),
        case("unrelated destination", 4, &[0x44; 6], None, 0, 0x50),
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
    cases.push(case(
        "VMACs precede payload and MU",
        15,
        &fields,
        Some([0x22; 6]),
        0,
        0x50,
    ));
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
            cases.push(invalid);
        }
    }
    cases
}
