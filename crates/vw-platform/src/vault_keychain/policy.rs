//! Platform-independent Keychain decisions, used by the native adapter and unit tests.
//! No access-change operation exists at this boundary; a failed read cannot trigger deletion.
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Binding {
    Bound,
    Legacy,
}

pub(super) fn classify(bound: bool, migrated: bool) -> Result<Binding, String> {
    if bound {
        Ok(Binding::Bound)
    } else if migrated {
        Err("Keychain item no longer trusts this app; unlock with the vault password".to_owned())
    } else {
        Ok(Binding::Legacy)
    }
}

// Cache failures, never secrets. Startup may ask multiple times through independent store instances;
// a refused prompt must not be presented again until restart or explicit removal of that credential.
pub(super) struct Attempts(std::collections::BTreeMap<(String, String), String>);
impl Attempts {
    pub(super) const fn new() -> Self {
        Self(std::collections::BTreeMap::new())
    }
    pub(super) fn run<T>(
        &mut self,
        service: &str,
        account: &str,
        call: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let key = (service.to_owned(), account.to_owned());
        if let Some(error) = self.0.get(&key) {
            return Err(error.clone());
        }
        let result = call();
        if let Err(error) = &result {
            self.0.insert(key, error.clone());
        }
        result
    }
    pub(super) fn removed(&mut self, service: &str, account: &str) {
        self.0.remove(&(service.to_owned(), account.to_owned()));
    }
}

pub(super) trait Keychain {
    fn inspect(&mut self) -> Result<Option<Binding>, String>;
    fn read(&mut self) -> Result<Zeroizing<Vec<u8>>, String>;
    // Prepare the access object before deleting anything; creation also records migration atomically.
    fn prepare_create(&mut self) -> Result<(), String>;
    fn create_bound(&mut self, secret: &[u8]) -> Result<(), String>;
    fn update(&mut self, secret: &[u8]) -> Result<(), String>;
    fn delete(&mut self) -> Result<(), String>;
}

pub(super) fn get(store: &mut impl Keychain) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    let Some(binding) = store.inspect()? else {
        return Ok(None);
    };
    let secret = store.read()?;
    if binding == Binding::Legacy {
        store.prepare_create()?;
        store.delete()?;
        store.create_bound(&secret)?;
    }
    Ok(Some(secret))
}

pub(super) fn set(store: &mut impl Keychain, secret: &[u8]) -> Result<(), String> {
    match store.inspect()? {
        Some(Binding::Bound) => store.update(secret),
        binding => {
            if binding == Some(Binding::Legacy) {
                // The old value must be authorized before replacement, even on a password unlock.
                let _authorized = store.read()?;
            }
            store.prepare_create()?;
            if binding.is_some() {
                store.delete()?;
            }
            store.create_bound(secret)
        }
    }
}

#[cfg(test)]
mod tests;
