//! Read-only ACL inspection and initial singleton access construction for the native vault store.
//! Signed apps are compared by designated requirement, not path or cdhash. Security.framework still
//! verifies the caller's signature on every read; inspecting identity never authorizes secret access.
use super::ffi::{self, Object, check};
use core_foundation::{
    array::CFArray,
    base::{CFType, TCFType},
    data::CFData,
    string::CFString,
};
use std::ptr;

pub(super) fn owned(object: Object) -> Result<CFType, String> {
    if object.is_null() {
        return Err("Keychain returned no object".to_owned());
    }
    // SAFETY: callers pass successful Copy/Create results, each adopted exactly once.
    Ok(unsafe { CFType::wrap_under_create_rule(object) })
}

fn objects(object: Object) -> Result<CFArray<CFType>, String> {
    let array = owned(object)?
        .downcast::<CFArray>()
        .ok_or_else(|| "Keychain returned an invalid array".to_owned())?;
    // SAFETY: these Security APIs return arrays of CF objects (ACLs, strings or trusted apps).
    Ok(unsafe { CFArray::wrap_under_get_rule(array.as_concrete_TypeRef()) })
}

fn current_app() -> Result<CFType, String> {
    let mut app = ptr::null();
    // SAFETY: NULL means this executable; the returned reference is owned.
    check(unsafe { ffi::SecTrustedApplicationCreateFromPath(ptr::null(), &raw mut app) })?;
    owned(app)
}

pub(super) fn initial(label: &str) -> Result<CFType, String> {
    let apps = CFArray::from_CFTypes(&[current_app()?]);
    let description = CFString::new(label);
    let mut access = ptr::null();
    // SAFETY: all inputs are live CF objects. An explicit singleton never allows every application.
    check(unsafe {
        ffi::SecAccessCreate(
            description.as_CFTypeRef(),
            apps.as_CFTypeRef(),
            &raw mut access,
        )
    })?;
    owned(access)
}

fn app_data(app: Object) -> Result<CFData, String> {
    let mut data = ptr::null();
    // SAFETY: app is a TrustedApplication from the system's ACL or current_app().
    check(unsafe { ffi::SecTrustedApplicationCopyData(app, &raw mut data) })?;
    owned(data)?
        .downcast()
        .ok_or_else(|| "Keychain returned invalid app data".to_owned())
}

fn requirement_data(app: Object) -> Result<Option<CFData>, String> {
    let mut requirement = ptr::null();
    // SAFETY: app comes from the system's ACL or current_app(); success may return NULL for unsigned code.
    check(unsafe { ffi::SecTrustedApplicationCopyRequirement(app, &raw mut requirement) })?;
    if requirement.is_null() {
        return Ok(None);
    }
    let requirement = owned(requirement)?;
    let mut data = ptr::null();
    // SAFETY: the requirement is live; flags=0 requests its canonical binary representation.
    check(unsafe { ffi::SecRequirementCopyData(requirement.as_CFTypeRef(), 0, &raw mut data) })?;
    owned(data)?
        .downcast()
        .map(Some)
        .ok_or_else(|| "Keychain returned invalid signing requirements".to_owned())
}

pub(super) fn is_bound(item: Object) -> Result<bool, String> {
    let this_app = current_app()?;
    let this_requirement = requirement_data(this_app.as_CFTypeRef())?;
    let mut access = ptr::null();
    // SAFETY: item is live; Copy returns an owned access object without changing it.
    check(unsafe { ffi::SecKeychainItemCopyAccess(item, &raw mut access) })?;
    let access = owned(access)?;
    let mut list = ptr::null();
    // SAFETY: access is live and the output receives an owned array of ACLs.
    check(unsafe { ffi::SecAccessCopyACLList(access.as_CFTypeRef(), &raw mut list) })?;
    let list = objects(list)?;
    let mut has_read_acl = false;
    for acl in list.iter() {
        // SAFETY: ACLs come from SecAccessCopyACLList; authorization strings are borrowed constants.
        let auths = objects(unsafe { ffi::SecACLCopyAuthorizations(acl.as_CFTypeRef()) })?;
        let decrypt = unsafe { CFType::wrap_under_get_rule(ffi::kSecACLAuthorizationDecrypt) };
        let any = unsafe { CFType::wrap_under_get_rule(ffi::kSecACLAuthorizationAny) };
        if !auths.iter().any(|auth| *auth == decrypt || *auth == any) {
            continue;
        }
        has_read_acl = true;
        let mut apps = ptr::null();
        let mut description = ptr::null();
        let mut selector = 0;
        // SAFETY: output pointers are valid. CopyContents requires all three even if unused.
        check(unsafe {
            ffi::SecACLCopyContents(
                acl.as_CFTypeRef(),
                &raw mut apps,
                &raw mut description,
                &raw mut selector,
            )
        })?;
        if !description.is_null() {
            drop(owned(description)?);
        }
        // A NULL application list permits everyone; an empty list always needs consent.
        if apps.is_null() {
            return Ok(false);
        }
        let apps = objects(apps)?;
        if apps.len() != 1 || selector & 1 != 0 {
            return Ok(false);
        }
        for app in apps.iter() {
            let matches = match (&this_requirement, requirement_data(app.as_CFTypeRef())?) {
                (Some(current), Some(stored)) => *current == stored,
                (None, None) => {
                    // Unsigned development executables have no DR. The OS still checks the legacy hash.
                    app_data(app.as_CFTypeRef())? == app_data(this_app.as_CFTypeRef())?
                }
                _ => false,
            };
            if !matches {
                return Ok(false);
            }
        }
    }
    Ok(has_read_acl)
}
