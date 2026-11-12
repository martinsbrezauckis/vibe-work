//! User-domain generic-password adapter for the vault policy. Depends on Security.framework and CF ownership.
//! Secrets are zeroed before freeing native buffers. Creation commits access and the migration marker together.
use super::{
    access,
    ffi::{self, Attribute, Attributes, check, length},
    policy::{Binding, Keychain, classify},
};
use core_foundation::base::{CFType, TCFType};
use std::{ffi::c_void, ptr};
use zeroize::{Zeroize, Zeroizing};

const MARKER: &[u8] = b"vibe-work-app-access-v1";
const COMMENT: u32 = u32::from_be_bytes(*b"icmt");

pub(super) struct Store<'a> {
    keychain: CFType,
    item: Option<CFType>,
    legacy: Option<CFType>,
    initial_access: Option<CFType>,
    service: String,
    account: &'a str,
    label: &'a str,
}

impl<'a> Store<'a> {
    pub(super) fn open(service: &'a str, account: &'a str, label: &'a str) -> Result<Self, String> {
        let mut keychain = ptr::null();
        // SAFETY: zero selects the User domain used by the previous keyring store.
        check(unsafe { ffi::SecKeychainCopyDomainDefault(0, &raw mut keychain) })?;
        let keychain = access::owned(keychain)?;
        // Generic passwords are unique by service/account. A distinct service lets the old item
        // survive creation failures and process exits. Always prefer the committed replacement.
        let migrated_service = format!("{service}.app-access-v1");
        let migrated = find(&keychain, &migrated_service, account)?;
        let original = find(&keychain, service, account)?;
        let (service, item, legacy) = if migrated.is_some() {
            (migrated_service, migrated, original)
        } else {
            (service.to_owned(), original, None)
        };
        Ok(Self {
            keychain,
            item,
            legacy,
            initial_access: None,
            service,
            account,
            label,
        })
    }

    pub(super) fn delete(&mut self) -> Result<(), String> {
        // Remove the fallback first: a partial failure must leave the preferred item usable.
        delete_item(&mut self.legacy)?;
        delete_item(&mut self.item)
    }

    fn item(&self) -> Result<&CFType, String> {
        self.item
            .as_ref()
            .ok_or_else(|| "Keychain item is missing".to_owned())
    }

    fn was_migrated(&self) -> Result<bool, String> {
        let mut comment = Attribute {
            tag: COMMENT,
            length: 0,
            data: ptr::null_mut(),
        };
        let mut attrs = Attributes {
            count: 1,
            attrs: &raw mut comment,
        };
        // SAFETY: the caller supplies the attribute descriptors; Security allocates their data.
        let status = unsafe {
            ffi::SecKeychainItemCopyContent(
                self.item()?.as_CFTypeRef(),
                ptr::null_mut(),
                &raw mut attrs,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if status == -25303 {
            return Ok(false);
        } // errSecNoSuchAttr: an older item without a comment
        check(status)?;
        let migrated = if comment.data.is_null() {
            false
        } else {
            // SAFETY: CopyContent initialized this buffer and length for the requested attribute.
            unsafe {
                std::slice::from_raw_parts(comment.data.cast::<u8>(), comment.length as usize)
                    == MARKER
            }
        };
        // SAFETY: frees only the attribute data returned by CopyContent, not the stack descriptors.
        check(unsafe { ffi::SecKeychainItemFreeContent(&raw mut attrs, ptr::null_mut()) })?;
        Ok(migrated)
    }
}

impl Keychain for Store<'_> {
    fn inspect(&mut self) -> Result<Option<Binding>, String> {
        let Some(item) = &self.item else {
            return Ok(None);
        };
        let bound = access::is_bound(item.as_CFTypeRef())?;
        // A changed ACL must not cause a migration loop or an allow-all read.
        classify(bound, !bound && self.was_migrated()?).map(Some)
    }

    fn read(&mut self) -> Result<Zeroizing<Vec<u8>>, String> {
        let mut size = 0;
        let mut bytes: *mut c_void = ptr::null_mut();
        // SAFETY: live item; Security allocates the returned secret only after authorizing this executable.
        check(unsafe {
            ffi::SecKeychainItemCopyContent(
                self.item()?.as_CFTypeRef(),
                ptr::null_mut(),
                ptr::null_mut(),
                &raw mut size,
                &raw mut bytes,
            )
        })?;
        let secret = if bytes.is_null() {
            if size != 0 {
                return Err("Keychain returned no secret data".to_owned());
            }
            Zeroizing::new(Vec::new())
        } else {
            // SAFETY: successful CopyContent returned size initialized bytes, owned until FreeContent.
            let buffer =
                unsafe { std::slice::from_raw_parts_mut(bytes.cast::<u8>(), size as usize) };
            let secret = Zeroizing::new(buffer.to_vec());
            buffer.zeroize();
            // SAFETY: this is the original allocation returned by CopyContent.
            check(unsafe { ffi::SecKeychainItemFreeContent(ptr::null_mut(), bytes) })?;
            secret
        };
        Ok(secret)
    }

    fn prepare_create(&mut self) -> Result<(), String> {
        self.initial_access = Some(access::initial(self.label)?);
        Ok(())
    }

    fn create_bound(&mut self, secret: &[u8]) -> Result<(), String> {
        let access = self
            .initial_access
            .as_ref()
            .ok_or_else(|| "Keychain access was not prepared".to_owned())?;
        let service = if self.item.is_some() {
            format!("{}.app-access-v1", self.service)
        } else {
            self.service.clone()
        };
        let mut fields = [
            attribute(*b"svce", service.as_bytes())?,
            attribute(*b"acct", self.account.as_bytes())?,
            attribute(*b"labl", self.label.as_bytes())?,
            attribute(*b"icmt", MARKER)?,
        ];
        let attrs = Attributes {
            count: 4,
            attrs: fields.as_mut_ptr(),
        };
        let mut item = ptr::null();
        // SAFETY: all input buffers remain live; initial access is installed as part of creation.
        // A duplicate is an error, never a reason to delete/retry another process's item.
        check(unsafe {
            ffi::SecKeychainItemCreateFromContent(
                u32::from_be_bytes(*b"genp"),
                &raw const attrs,
                length(secret)?,
                secret.as_ptr().cast(),
                self.keychain.as_CFTypeRef(),
                access.as_CFTypeRef(),
                &raw mut item,
            )
        })?;
        let created = access::owned(item)?;
        self.legacy = self.item.replace(created);
        self.service = service;
        Ok(())
    }

    fn update(&mut self, secret: &[u8]) -> Result<(), String> {
        // SAFETY: NULL attributes updates only the secret, preserving ACL and migration marker.
        check(unsafe {
            ffi::SecKeychainItemModifyContent(
                self.item()?.as_CFTypeRef(),
                ptr::null(),
                length(secret)?,
                secret.as_ptr().cast(),
            )
        })
    }

    fn delete_legacy(&mut self) -> Result<(), String> {
        delete_item(&mut self.legacy)
    }
}

fn attribute(tag: [u8; 4], bytes: &[u8]) -> Result<Attribute, String> {
    Ok(Attribute {
        tag: u32::from_be_bytes(tag),
        length: length(bytes)?,
        data: bytes.as_ptr().cast_mut().cast(),
    })
}

fn find(keychain: &CFType, service: &str, account: &str) -> Result<Option<CFType>, String> {
    let mut item = ptr::null();
    // SAFETY: live slices and checked lengths; null password outputs request metadata only.
    let status = unsafe {
        ffi::SecKeychainFindGenericPassword(
            keychain.as_CFTypeRef(),
            length(service.as_bytes())?,
            service.as_ptr().cast(),
            length(account.as_bytes())?,
            account.as_ptr().cast(),
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut item,
        )
    };
    if status == -25300 {
        return Ok(None);
    }
    check(status)?;
    access::owned(item).map(Some)
}

fn delete_item(item: &mut Option<CFType>) -> Result<(), String> {
    if let Some(found) = item {
        // SAFETY: deletes the exact retained item, never a name-based second lookup.
        check(unsafe { ffi::SecKeychainItemDelete(found.as_CFTypeRef()) })?;
        *item = None;
    }
    Ok(())
}
