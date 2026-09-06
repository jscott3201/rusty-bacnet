use super::AuditLogQueryAck;
use crate::common::{decode_context, decode_context_bool, MAX_DECODED_ITEMS};
use bacnet_encoding::constructed::{
    decode_audit_log_record_result_at, encode_audit_log_record_result,
};
use bacnet_encoding::{primitives, tags};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

const AUDIT_LOG_TAG: u8 = 0;
const RECORDS_TAG: u8 = 1;
const NO_MORE_ITEMS_TAG: u8 = 2;

pub(super) fn encode(ack: &AuditLogQueryAck, buf: &mut BytesMut) -> Result<(), Error> {
    if ack.records.len() > MAX_DECODED_ITEMS {
        return Err(Error::OutOfRange(format!(
            "AuditLogQuery-ACK record count {} exceeds {MAX_DECODED_ITEMS}",
            ack.records.len()
        )));
    }

    let mut encoded = BytesMut::new();
    primitives::encode_ctx_object_id(&mut encoded, AUDIT_LOG_TAG, &ack.audit_log);
    tags::encode_opening_tag(&mut encoded, RECORDS_TAG);
    for result in &ack.records {
        encode_audit_log_record_result(result, &mut encoded)?;
    }
    tags::encode_closing_tag(&mut encoded, RECORDS_TAG);
    primitives::encode_ctx_boolean(&mut encoded, NO_MORE_ITEMS_TAG, ack.no_more_items);
    buf.extend_from_slice(&encoded);
    Ok(())
}

pub(super) fn decode(data: &[u8]) -> Result<AuditLogQueryAck, Error> {
    let (audit_log_contents, mut offset) =
        decode_context(data, 0, AUDIT_LOG_TAG, "AuditLogQuery-ACK audit-log")?;
    let audit_log = ObjectIdentifier::decode(audit_log_contents)?;

    let (records_body, records_end) =
        decode_constructed_body(data, offset, RECORDS_TAG, "record list")?;
    let mut records = Vec::new();
    let mut record_offset = 0;
    while record_offset < records_body.len() {
        if records.len() >= MAX_DECODED_ITEMS {
            return Err(Error::decoding(
                offset + record_offset,
                format!("AuditLogQuery-ACK record count exceeds {MAX_DECODED_ITEMS}"),
            ));
        }
        let (result, next) = decode_audit_log_record_result_at(records_body, record_offset)?;
        if next <= record_offset {
            return Err(Error::decoding(
                offset + record_offset,
                "AuditLogQuery-ACK record decoder made no progress",
            ));
        }
        records.push(result);
        record_offset = next;
    }
    offset = records_end;

    let (no_more_items, end) = decode_context_bool(
        data,
        offset,
        NO_MORE_ITEMS_TAG,
        "AuditLogQuery-ACK no-more-items",
    )?;
    if end != data.len() {
        return Err(Error::decoding(
            end,
            "AuditLogQuery-ACK has trailing service data",
        ));
    }

    Ok(AuditLogQueryAck {
        audit_log,
        records,
        no_more_items,
    })
}

fn decode_constructed_body<'a>(
    data: &'a [u8],
    offset: usize,
    tag_number: u8,
    field: &str,
) -> Result<(&'a [u8], usize), Error> {
    let (opening, body_start) = tags::decode_tag(data, offset)?;
    if !opening.is_opening_tag(tag_number) {
        return Err(Error::decoding(
            offset,
            format!("AuditLogQuery-ACK {field} expected opening tag [{tag_number}]"),
        ));
    }
    tags::extract_context_value(data, body_start, tag_number)
}
