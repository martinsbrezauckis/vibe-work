//! How a paired profile is named on screen and in agent text: the browser's own name, the signed-in account's
//! domain in agent text, and the pairing time or number when two profiles would otherwise read the same.
use super::{Browser, Profiles, canonical, paired_label, usable_metadata};

impl Profiles {
    /// The shown name. Profiles shown with the same name and the same signed-in account (or none) carry when each
    /// was paired ("Safari — paired 12 Nov 23:25"), with seconds when the minute is shared; only identical pairing
    /// times fall back to their order. Different accounts tell same-named profiles apart: the screen shows the
    /// account next to the name.
    pub(in crate::chrome_control) fn name(&self, id: &str) -> String {
        self.shown_name(id, false)
    }
    /// The profile's name in agent text: the shown name with the signed-in account's domain after the browser's name,
    /// "Alex (company.example)" or "Work (company.example) — paired 12 Nov 23:25". No domain without an account, and
    /// none when the name already holds it (a managed profile the browser named "company.example", or a name typed as
    /// an email, which is what the browser shows). The account itself never reaches agent text.
    pub(in crate::chrome_control) fn agent_name(&self, id: &str) -> String {
        self.shown_name(id, true)
    }
    /// The shown name, with the account's domain when `domain`. Profiles whose shown part is equal (the name and
    /// account on screen, the name and domain in agent text) are told apart as in `name`.
    fn shown_name(&self, id: &str, domain: bool) -> String {
        let id = canonical(id);
        let base = self.base_name(&id);
        let head = if domain {
            self.with_domain(&id, &base)
        } else {
            base.clone()
        };
        let browser = Browser::from_profile(&id);
        let mut twins: Vec<&String> = self
            .records
            .iter()
            .filter(|(other, _)| {
                *other != &id
                    && self.shown(other)
                    && browser.owns_profile(other)
                    && self.base_name(other) == base
                    && if domain {
                        self.with_domain(other, &base) == head
                    } else {
                        self.account(other).map(|a| a.to_lowercase())
                            == self.account(&id).map(|a| a.to_lowercase())
                    }
            })
            .map(|(other, _)| other)
            .collect();
        if twins.is_empty() {
            return head;
        }
        let at = |id: &str| self.records.get(id).and_then(|record| record.paired_at);
        for seconds in [false, true] {
            let mine = paired_label(at(&id), seconds);
            twins.retain(|other| paired_label(at(other), seconds) == mine);
            if twins.is_empty() {
                return format!("{head} — {mine}");
            }
        }
        let mine = paired_label(at(&id), true);
        let order = |id: &str| {
            (
                self.records.get(id).and_then(|record| record.number),
                id.to_owned(),
            )
        };
        let position = twins
            .iter()
            .filter(|other| order(other) < order(&id))
            .count()
            .saturating_add(1);
        format!("{head} — {mine} ({position})")
    }
    /// `name` followed by the account's domain in brackets, unless there is none or `name` already holds it.
    fn with_domain(&self, id: &str, name: &str) -> String {
        match self.domain(id) {
            Some(domain) if !name.to_lowercase().contains(&domain) => format!("{name} ({domain})"),
            _ => name.to_owned(),
        }
    }
    /// The domain of the signed-in account ("company.example" for "a@Company.Example"), lower case; none without an
    /// account or when the part after '@' is not a domain name.
    fn domain(&self, id: &str) -> Option<String> {
        let account = self.records.get(id)?.account.as_deref()?;
        let domain = account.rsplit_once('@')?.1.trim().to_lowercase();
        (domain.len() <= 253
            && domain.contains('.')
            && domain.split('.').all(|part| {
                !part.is_empty() && part.chars().all(|c| c.is_alphanumeric() || c == '-')
            }))
        .then_some(domain)
    }
    /// The browser's name; else an earlier saved or helper-reported name; else browser and pairing order.
    fn base_name(&self, id: &str) -> String {
        let record = self.records.get(id);
        record
            .and_then(|record| {
                record
                    .browser_name
                    .as_ref()
                    .or(record.name.as_ref())
                    .or(usable_metadata(record))
            })
            .cloned()
            .unwrap_or_else(|| self.numbered_name(id))
    }
    /// Browser and pairing number: "Chrome profile 2". The number stays with the profile, so a name the person or
    /// the agent already saw keeps meaning the same profile: removing another profile renumbers nothing, and a newer
    /// identity proven in the same profile folder (the folder paired again) keeps the folder's first number. Safari
    /// and Firefox are named by the browser alone ("Safari"); `name` tells several apart by when each was paired.
    pub(super) fn numbered_name(&self, id: &str) -> String {
        let browser = Browser::from_profile(id);
        if matches!(browser, Browser::Safari | Browser::Firefox) {
            return browser.short_name().into();
        }
        let number = self.records.get(id).and_then(|record| {
            let same_folder = self
                .records
                .iter()
                .filter(|(other, older)| {
                    browser.owns_profile(other)
                        && record.folder.is_some()
                        && older.folder == record.folder
                })
                .filter_map(|(_, older)| older.number);
            same_folder.chain(record.number).min()
        });
        let browser = browser.short_name();
        match number {
            Some(number) => format!("{browser} profile {number}"),
            None => format!("{browser} profile"),
        }
    }
}
