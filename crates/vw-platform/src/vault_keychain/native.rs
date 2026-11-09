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
    initial_access: Option<CFType>,
    service: &'a str,
    account: &'a str,
    label: &'a str,
}

impl<'a> Store<'a> {
    pub(super) fn open(service: &'a str, account: &'a str, label: &'a str) -> Result<Self, String> {
        let service_len = length(service.as_bytes())?;
        let account_len = length(account.as_bytes())?;
        let mut keychain = ptr::null();
        // SAFETY: zero selects the User domain used by the previous keyring store.
        check(unsafe { ffi::SecKeychainCopyDomainDefault(0, &raw mut keychain) })?;
        let keychain = access::owned(keychain)?;
        let mut item = ptr::null();
        // SAFETY: live byte slices with checked lengths; NULL password outputs find metadata only.
        let status = unsafe {
            ffi::SecKeychainFindGenericPassword(
                keychain.as_CFTypeRef(),
                service_len,
                service.as_ptr().cast(),
                account_len,
                account.as_ptr().cast(),
                ptr::null_mut(),
                ptr::null_mut(),
                &raw mut item,
            )
        };
        let item = if status == -25300 {
            None
        } else {
            check(status)?;
            Some(access::owned(item)?)
        };
        Ok(Self {
            keychain,
            item,
            initial_access: None,
            service,
            account,
            label,
        })
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
        let mut fields = [
            attribute(*b"svce", self.service.as_bytes())?,
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
        self.item = Some(access::owned(item)?);
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

    fn delete(&mut self) -> Result<(), String> {
        if let Some(item) = &self.item {
            // SAFETY: this is the exact item found in the selected keychain; no name-based second lookup.
            check(unsafe { ffi::SecKeychainItemDelete(item.as_CFTypeRef()) })?;
            self.item = None;
        }
        Ok(())
    }
}

fn attribute(tag: [u8; 4], bytes: &[u8]) -> Result<Attribute, String> {
    Ok(Attribute {
        tag: u32::from_be_bytes(tag),
        length: length(bytes)?,
        data: bytes.as_ptr().cast_mut().cast(),
    })
}
