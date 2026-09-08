//! ROOT-1 semantic characterization; no wall-clock performance assertions.
use super::*;

const REQUIRED_COUNT: usize = 4096;
const OPTIONAL_IDS: [PropertyIdentifier; 4] = [
    PropertyIdentifier::from_raw(6001),
    PropertyIdentifier::from_raw(6000),
    PropertyIdentifier::from_raw(6001),
    PropertyIdentifier::from_raw(6002),
];

const fn properties() -> [PropertyIdentifier; REQUIRED_COUNT + 4] {
    let mut result = [PropertyIdentifier::PRESENT_VALUE; REQUIRED_COUNT + 4];
    let mut i = 0;
    while i < REQUIRED_COUNT {
        result[i] = PropertyIdentifier::from_raw(512 + i as u32);
        i += 1;
    }
    let mut j = 0;
    while j < OPTIONAL_IDS.len() {
        result[i + j] = OPTIONAL_IDS[j];
        j += 1;
    }
    result
}

static PROPERTIES: [PropertyIdentifier; REQUIRED_COUNT + 4] = properties();

struct BorrowedLegacy {
    all_required: bool,
    reads: Arc<AtomicUsize>,
}

impl BACnetObject for BorrowedLegacy {
    fn object_identifier(&self) -> ObjectIdentifier {
        oid(1)
    }
    fn object_name(&self) -> &str {
        "large borrowed legacy metadata"
    }
    // Deliberately inherit the empty property_metadata default.
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(if self.all_required {
            &PROPERTIES[..REQUIRED_COUNT]
        } else {
            &PROPERTIES
        })
    }
    fn required_properties(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&PROPERTIES[..REQUIRED_COUNT])
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        _: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(PropertyValue::Unsigned(u64::from(property.to_raw())))
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        unreachable!()
    }
}

fn legacy_fixture(all_required: bool) -> (ObjectDatabase, Arc<AtomicUsize>) {
    let reads = Arc::new(AtomicUsize::new(0));
    let mut db = ObjectDatabase::new();
    db.add(Box::new(BorrowedLegacy {
        all_required,
        reads: reads.clone(),
    }))
    .unwrap();
    (db, reads)
}

#[test]
fn rpm_optional_large_borrowed_all_required_preserves_empty_wrappers() {
    let (db, reads) = legacy_fixture(true);
    let data = request(vec![
        spec(1, vec![reference(PropertyIdentifier::OPTIONAL)]),
        spec(1, vec![reference(PropertyIdentifier::OPTIONAL)]),
    ]);
    for bytes in [13, 14, 15] {
        let mut out = BytesMut::from(&b"prefix"[..]);
        let result = handle_rpm_budgeted(&db, &data, &mut out, budget(1, bytes));
        if bytes == 13 {
            assert!(matches!(result, Err(RpmFailure::Bytes)));
            assert_eq!(&out[..], b"prefix");
        } else {
            result.unwrap();
            let empty = [0x0c, 0, 0, 0, 1, 0x1e, 0x1f];
            assert_eq!(&out[6..], empty.repeat(2));
        }
    }
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[test]
fn rpm_optional_large_borrowed_mostly_required_order_duplicates_and_late_cap() {
    let (db, reads) = legacy_fixture(false);
    let object = db.get(&oid(1)).unwrap();
    assert!(object.property_metadata().is_empty());
    assert!(matches!(object.property_list(), Cow::Borrowed(_)));
    assert!(matches!(object.required_properties(), Cow::Borrowed(_)));

    // The visitor's failure must stop before the final optional occurrence;
    // required rows themselves never spend expanded-result capacity.
    let mut visits = Vec::new();
    let result = expand(object, &reference(PropertyIdentifier::OPTIONAL), |id| {
        visits.push(id);
        if visits.len() == 3 {
            Err(RpmFailure::Work)
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Err(RpmFailure::Work)));
    assert_eq!(visits, OPTIONAL_IDS[..3]);

    let data = request(vec![
        spec(1, vec![reference(PROPERTIES[0])]),
        spec(1, vec![reference(PropertyIdentifier::OPTIONAL)]),
    ]);
    let mut out = BytesMut::from(&b"prefix"[..]);
    assert!(matches!(
        handle_rpm_budgeted(&db, &data, &mut out, budget(4, 1024)),
        Err(RpmFailure::Work)
    ));
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(&out[..], b"prefix");

    for limit in [5, 6] {
        let mut out = BytesMut::new();
        handle_rpm_budgeted(&db, &data, &mut out, budget(limit, 1024)).unwrap();
        let ack = ReadPropertyMultipleACK::decode(&out).unwrap();
        let results = &ack.list_of_read_access_results[1].list_of_results;
        assert_eq!(
            results
                .iter()
                .map(|r| r.property_identifier)
                .collect::<Vec<_>>(),
            OPTIONAL_IDS
        );
        let mut legacy = BytesMut::new();
        handle_read_property_multiple(&db, &data, &mut legacy).unwrap();
        assert_eq!(out, legacy);
    }
}
