//! Security.framework ABI used only by the vault Keychain adapter. Signatures follow Apple's headers.
//! Owned Copy/Create results are released by the callers; no function here changes existing access.
use std::ffi::{c_char, c_void};
pub(super) type Object = *const c_void;

#[repr(C)]
pub(super) struct Attribute {
    pub tag: u32,
    pub length: u32,
    pub data: *mut c_void,
}
#[repr(C)]
pub(super) struct Attributes {
    pub count: u32,
    pub attrs: *mut Attribute,
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    pub fn SecKeychainCopyDomainDefault(domain: i32, keychain: *mut Object) -> i32;
    pub fn SecKeychainFindGenericPassword(
        keychain: Object,
        service_len: u32,
        service: *const c_char,
        account_len: u32,
        account: *const c_char,
        password_len: *mut u32,
        password: *mut *mut c_void,
        item: *mut Object,
    ) -> i32;
    pub fn SecTrustedApplicationCreateFromPath(path: *const c_char, app: *mut Object) -> i32;
    pub fn SecTrustedApplicationCopyData(app: Object, data: *mut Object) -> i32;
    // Exported legacy SPI declared in Apple's SecTrustedApplicationPriv.h. CopyData alone is only
    // a path, not signing identity; comparing requirements avoids rebinding on signed app updates.
    pub fn SecTrustedApplicationCopyRequirement(app: Object, requirement: *mut Object) -> i32;
    pub fn SecRequirementCopyData(requirement: Object, flags: u32, data: *mut Object) -> i32;
    pub fn SecAccessCreate(description: Object, applications: Object, access: *mut Object) -> i32;
    pub fn SecKeychainItemCopyAccess(item: Object, access: *mut Object) -> i32;
    pub fn SecAccessCopyACLList(access: Object, list: *mut Object) -> i32;
    pub fn SecACLCopyAuthorizations(acl: Object) -> Object;
    pub fn SecACLCopyContents(
        acl: Object,
        apps: *mut Object,
        description: *mut Object,
        selector: *mut u16,
    ) -> i32;
    pub static kSecACLAuthorizationDecrypt: Object;
    pub static kSecACLAuthorizationAny: Object;
    pub fn SecKeychainItemCreateFromContent(
        class: u32,
        attrs: *const Attributes,
        length: u32,
        data: *const c_void,
        keychain: Object,
        access: Object,
        item: *mut Object,
    ) -> i32;
    pub fn SecKeychainItemCopyContent(
        item: Object,
        class: *mut u32,
        attrs: *mut Attributes,
        length: *mut u32,
        data: *mut *mut c_void,
    ) -> i32;
    pub fn SecKeychainItemFreeContent(attrs: *mut Attributes, data: *mut c_void) -> i32;
    pub fn SecKeychainItemModifyContent(
        item: Object,
        attrs: *const Attributes,
        length: u32,
        data: *const c_void,
    ) -> i32;
    pub fn SecKeychainItemDelete(item: Object) -> i32;
}

pub(super) fn check(code: i32) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!("Keychain operation failed ({code})"))
    }
}
pub(super) fn length(bytes: &[u8]) -> Result<u32, String> {
    u32::try_from(bytes.len()).map_err(|_| "Keychain value is too long".to_owned())
}
