//! Independent Annex AB.2.10–11 wire fixtures.

pub(crate) fn valid_connect(function: u8, vmac: [u8; 6]) -> Vec<u8> {
    let mut wire = vec![function, 0, 0x22, 0x33];
    wire.extend_from_slice(&vmac);
    wire.extend_from_slice(&[0x33; 16]);
    // Peer receive capacities can exceed our own 1476-byte receive cap.
    wire.extend_from_slice(&[0x20, 0x00, 0x10, 0x00]);
    wire
}

pub(crate) struct InvalidConnect {
    pub name: &'static str,
    pub wire: Vec<u8>,
    pub nak: Option<Vec<u8>>,
}

fn case(
    name: &'static str,
    control: u8,
    fields: &[u8],
    payload: &[u8],
    reply: Option<[u8; 6]>,
    marker: u8,
    code: u8,
) -> InvalidConnect {
    let mut wire = vec![6, control, 0x22, 0x33];
    wire.extend_from_slice(fields);
    wire.extend_from_slice(payload);
    let mut nak = vec![0, if reply.is_some() { 4 } else { 0 }, 0x22, 0x33];
    if let Some(reply) = reply {
        nak.extend_from_slice(&reply);
    }
    nak.extend_from_slice(&[6, 1, marker, 0, 7, 0, code]);
    InvalidConnect {
        name,
        wire,
        nak: Some(nak),
    }
}

pub(crate) fn invalid_connects(function: u8, local: [u8; 6]) -> Vec<InvalidConnect> {
    let valid = valid_connect(6, [0x22; 6]);
    let payload = &valid[4..];
    let mut cases = vec![
        case("source", 8, &[0x44; 6], payload, Some([0x44; 6]), 0, 80),
        case("local destination", 4, &local, payload, None, 0, 80),
        case("unrelated destination", 4, &[0x66; 6], payload, None, 0, 80),
        case("Data Options", 1, &[0x1E], payload, None, 0, 80),
        case("unsupported MU", 2, &[0x5E], payload, None, 0x5E, 146),
        case(
            "first raw MU marker",
            2,
            &[0x9E, 0xFE, 0, 0, 0x5D],
            payload,
            None,
            0xFE,
            146,
        ),
    ];
    for (length, code) in [(0, 149), (1, 147), (25, 147), (27, 7)] {
        // Length errors also precede a reserved payload identity.
        let malformed = vec![0; length];
        cases.push(case("payload boundary", 0, &[], &malformed, None, 0, code));
        cases.push(case(
            "length precedes MU",
            2,
            &[0x5E],
            &malformed,
            None,
            0,
            code,
        ));
    }
    for reserved in [[0; 6], [0xff; 6]] {
        let mut malformed = payload.to_vec();
        malformed[..6].copy_from_slice(&reserved);
        cases.push(case(
            "reserved payload identity is not reply source",
            0,
            &[],
            &malformed,
            None,
            0,
            80,
        ));
        cases.push(case(
            "identity precedes MU",
            2,
            &[0x5E],
            &malformed,
            None,
            0,
            80,
        ));
    }
    // Explicit envelope addresses are independent of the payload identity.
    for source in [None, Some([0x44; 6]), Some([0; 6]), Some([0xff; 6])] {
        for destination in [None, Some(local), Some([0x66; 6]), Some([0xff; 6])] {
            if source.is_none() && destination.is_none() {
                continue;
            }
            let mut fields = Vec::new();
            if let Some(source) = source {
                fields.extend_from_slice(&source);
            }
            if let Some(destination) = destination {
                fields.extend_from_slice(&destination);
            }
            fields.extend_from_slice(&[0x5E, 0x1E]);
            let control = 3
                | if source.is_some() { 8 } else { 0 }
                | if destination.is_some() { 4 } else { 0 };
            // Missing payload and MU faults must lose to forbidden presence.
            let mut invalid = case(
                "envelope addresses and presence precedence",
                control,
                &fields,
                &[],
                source,
                0,
                80,
            );
            if source == Some([0; 6]) || source == Some([0xff; 6]) || destination == Some([0xff; 6])
            {
                invalid.nak = None;
            }
            cases.push(invalid);
        }
    }
    for invalid in &mut cases {
        invalid.wire[0] = function;
        if function == 7 {
            invalid.nak = None;
        }
    }
    cases
}

pub(crate) fn valid_connect_with_options(function: u8, vmac: [u8; 6]) -> Vec<u8> {
    let mut wire = valid_connect(function, vmac);
    wire[1] = 2;
    // Two well-formed, unknown MU-clear options of the same type.
    wire.splice(4..4, [0x9E, 0x3E, 0, 0]);
    wire
}
