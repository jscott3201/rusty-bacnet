//! Random-48 VMAC generation for BACnet/SC.

use bacnet_types::error::Error;

use crate::sc_frame::Vmac;

/// Generate an Annex H.7.3 Random-48 VMAC.
///
/// The low nibble of the first octet is fixed at X'2'; the high nibble and
/// remaining five octets carry 44 bits of OS randomness.
pub(crate) fn generate_random48_vmac() -> Result<Vmac, Error> {
    #[cfg(test)]
    if let Some(generator) = test_random48_vmac_generator() {
        return generator();
    }

    let mut vmac = [0u8; 6];
    getrandom::fill(&mut vmac)
        .map_err(|e| Error::Encoding(format!("failed to generate Random-48 VMAC: {e}")))?;
    vmac[0] = (vmac[0] & 0xF0) | 0x02;
    Ok(vmac)
}

#[cfg(test)]
thread_local! {
    static TEST_RANDOM48_VMAC_GENERATOR:
        std::cell::RefCell<Option<fn() -> Result<Vmac, Error>>> =
            const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn test_random48_vmac_generator() -> Option<fn() -> Result<Vmac, Error>> {
    TEST_RANDOM48_VMAC_GENERATOR.with(|generator| *generator.borrow())
}

#[cfg(test)]
pub(crate) struct TestRandom48VmacGeneratorGuard;

#[cfg(test)]
impl Drop for TestRandom48VmacGeneratorGuard {
    fn drop(&mut self) {
        TEST_RANDOM48_VMAC_GENERATOR.with(|generator| {
            *generator.borrow_mut() = None;
        });
    }
}

#[cfg(test)]
pub(crate) fn set_test_random48_vmac_generator(
    generator: fn() -> Result<Vmac, Error>,
) -> TestRandom48VmacGeneratorGuard {
    TEST_RANDOM48_VMAC_GENERATOR.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "test Random-48 generator already set"
        );
        *slot.borrow_mut() = Some(generator);
    });
    TestRandom48VmacGeneratorGuard
}
