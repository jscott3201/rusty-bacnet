//! Explicit, bounded endpoint RPM profile and complete ordered ACK correlation.
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_types::{enums::PropertyIdentifier, error::Error, primitives::ObjectIdentifier};

pub(crate) fn validate_request(request: &ReadPropertyMultipleRequest) -> Result<(), Error> {
    let mut count = 0usize;
    if request.list_of_read_access_specs.is_empty() {
        return Err(Error::Encoding(
            "endpoint RPM requires nonempty object specifications".into(),
        ));
    }
    for spec in &request.list_of_read_access_specs {
        if spec.object_identifier.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE
            || spec.list_of_property_references.is_empty()
        {
            return Err(Error::Encoding(
                "endpoint RPM requires concrete objects and nonempty references".into(),
            ));
        }
        count = count.saturating_add(spec.list_of_property_references.len());
        if count > 64 {
            return Err(Error::Encoding(
                "endpoint RPM supports at most 64 property references".into(),
            ));
        }
        if spec.list_of_property_references.iter().any(|p| {
            matches!(
                p.property_identifier,
                PropertyIdentifier::ALL
                    | PropertyIdentifier::REQUIRED
                    | PropertyIdentifier::OPTIONAL
            )
        }) {
            return Err(Error::Encoding(
                "endpoint RPM requires explicit property identifiers".into(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn decode_ack(
    request: &ReadPropertyMultipleRequest,
    bytes: &[u8],
) -> Result<ReadPropertyMultipleACK, Error> {
    let ack = ReadPropertyMultipleACK::decode(bytes)?;
    let mismatch = || Error::decoding(0, "RPM ACK does not match ordered request occurrences");
    if request.list_of_read_access_specs.len() != ack.list_of_read_access_results.len() {
        return Err(mismatch());
    }
    for (spec, result) in request
        .list_of_read_access_specs
        .iter()
        .zip(&ack.list_of_read_access_results)
    {
        if spec.object_identifier != result.object_identifier
            || spec.list_of_property_references.len() != result.list_of_results.len()
        {
            return Err(mismatch());
        }
        for (requested, reported) in spec
            .list_of_property_references
            .iter()
            .zip(&result.list_of_results)
        {
            let index_matches = match (
                requested.property_array_index,
                reported.property_array_index,
                reported.error,
            ) {
                (Some(_), None, Some(_)) => true,
                (expected, actual, _) => expected == actual,
            };
            if requested.property_identifier != reported.property_identifier || !index_matches {
                return Err(mismatch());
            }
        }
    }
    Ok(ack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_services::{
        common::PropertyReference,
        rpm::{ReadAccessResult, ReadAccessSpecification, ReadResultElement},
    };
    use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
    use bytes::BytesMut;
    fn request() -> ReadPropertyMultipleRequest {
        ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap(),
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: Some(0),
                }],
            }],
        }
    }
    #[test]
    fn endpoint_rpm_profile_bounds_and_standalone_codec_distinction() {
        let one = request();
        for count in [1, 64, 65] {
            let mut r = one.clone();
            r.list_of_read_access_specs[0].list_of_property_references = vec![
                    one.list_of_read_access_specs[0].list_of_property_references[0].clone();
                    count
                ];
            assert_eq!(validate_request(&r).is_ok(), count <= 64);
            assert!(r.encode(&mut BytesMut::new()).is_ok());
        }
        for property in [
            PropertyIdentifier::ALL,
            PropertyIdentifier::REQUIRED,
            PropertyIdentifier::OPTIONAL,
        ] {
            let mut r = one.clone();
            r.list_of_read_access_specs[0].list_of_property_references[0].property_identifier =
                property;
            assert!(validate_request(&r).is_err());
            assert!(r.encode(&mut BytesMut::new()).is_ok());
        }
        for kind in [
            ObjectType::DEVICE,
            ObjectType::NETWORK_PORT,
            ObjectType::ANALOG_INPUT,
        ] {
            let mut r = one.clone();
            r.list_of_read_access_specs[0].object_identifier =
                ObjectIdentifier::new(kind, ObjectIdentifier::MAX_INSTANCE).unwrap();
            assert!(validate_request(&r).is_err());
        }
        let mut r = one.clone();
        r.list_of_read_access_specs[0]
            .list_of_property_references
            .clear();
        assert!(validate_request(&r).is_err());
        r.list_of_read_access_specs.clear();
        assert!(validate_request(&r).is_err());
    }
    #[test]
    fn endpoint_rpm_inline_error_index_contract_is_independent_of_error_code() {
        for requested in [None, Some(0), Some(7)] {
            let mut request = request();
            request.list_of_read_access_specs[0].list_of_property_references[0]
                .property_array_index = requested;
            for actual in [None, Some(0), Some(7), Some(9)] {
                for error in [
                    None,
                    Some((ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)),
                    Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
                    Some((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)),
                ] {
                    let ack = ReadPropertyMultipleACK {
                        list_of_read_access_results: vec![ReadAccessResult {
                            object_identifier: request.list_of_read_access_specs[0]
                                .object_identifier,
                            list_of_results: vec![ReadResultElement {
                                property_identifier: PropertyIdentifier::OBJECT_NAME,
                                property_array_index: actual,
                                property_value: error.is_none().then(|| vec![0x21, 42]),
                                error,
                            }],
                        }],
                    };
                    let mut bytes = BytesMut::new();
                    ack.encode(&mut bytes);
                    let valid = actual == requested
                        || (requested.is_some() && actual.is_none() && error.is_some());
                    assert_eq!(
                        decode_ack(&request, &bytes).is_ok(),
                        valid,
                        "{requested:?} {actual:?} {error:?}"
                    );
                }
            }
        }
    }
}
