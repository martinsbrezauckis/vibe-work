//! Opt-in native smoke test of the real adapter with Keychain UI disabled for this test process.
//! Run alone in a signed test binary on an unlocked Mac login; only a random synthetic item is touched.
use super::{delete, get, set};

const SERVICE: &str = "vibe-work-keychain-test";

struct SyntheticItem(String);
impl Drop for SyntheticItem {
    fn drop(&mut self) {
        let _ = delete(SERVICE, &self.0);
    }
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecKeychainGetUserInteractionAllowed(allowed: *mut u8) -> i32;
    fn SecKeychainSetUserInteractionAllowed(allowed: u8) -> i32;
}

struct Interaction(u8);
impl Drop for Interaction {
    fn drop(&mut self) {
        // SAFETY: restores the process-local setting captured before this test.
        unsafe { SecKeychainSetUserInteractionAllowed(self.0) };
    }
}

#[test]
#[ignore = "needs an unlocked macOS login Keychain; run alone in a signed test executable"]
fn new_item_and_repeated_reads_and_writes_need_no_keychain_interaction() {
    let mut previous = 0;
    // SAFETY: valid output pointer and Boolean arguments; applies only to this disposable test process.
    assert_eq!(
        unsafe { SecKeychainGetUserInteractionAllowed(&raw mut previous) },
        0
    );
    let _restore = Interaction(previous);
    assert_eq!(unsafe { SecKeychainSetUserInteractionAllowed(0) }, 0);
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).unwrap();
    let account = format!("synthetic-{:032x}", u128::from_ne_bytes(random));
    let _cleanup = SyntheticItem(account.clone());
    let service = SERVICE;
    assert!(
        get(service, &account, "Synthetic vault test")
            .unwrap()
            .is_none()
    );
    let result = (|| -> Result<(), String> {
        set(
            service,
            &account,
            "Synthetic vault test",
            b"first synthetic secret",
        )?;
        for _ in 0..3 {
            assert_eq!(
                &*get(service, &account, "Synthetic vault test")?.unwrap(),
                b"first synthetic secret"
            );
        }
        set(
            service,
            &account,
            "Synthetic vault test",
            b"second synthetic secret",
        )?;
        assert_eq!(
            &*get(service, &account, "Synthetic vault test")?.unwrap(),
            b"second synthetic secret"
        );
        Ok(())
    })();
    let cleanup = delete(service, &account);
    result.unwrap();
    cleanup.unwrap();
    assert!(
        get(service, &account, "Synthetic vault test")
            .unwrap()
            .is_none()
    );
}
