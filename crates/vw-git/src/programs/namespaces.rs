//! Git and companion namespaces that must retain the publish allowlist.
//! Uses the documented configuration families plus Git's own runtime inventory; custom namespaces are data.

// Keep complete families restricted, including undocumented/future keys and companion tools' settings.
const RESERVED: &str = "add advice alias am apply attr author bitmappseudomerge blame branch browser bundle \
checkout clean clone color column commit commitgraph committer completion core credential credentialcache \
credentialstore diff difftool extensions fastimport feature fetch filter format fsck fsmonitor gc gitcvs \
gitweb gpg grep gui guitool help hook hooks http i18n imap include includeif index init instaweb interactive \
lfs log lsrefs mailinfo mailmap maintenance man merge mergetool notes pack pager pretty promisor protocol pull \
push rebase receive reftable remote remotes repack replay rerere revert safe scalar sendemail sequence showbranch \
sparse splitindex ssh stash submodule status tag tar trace2 trailer transfer uploadarchive uploadpack \
uploadpackfilter url user versionsort web worktree git-p4 p4 git-svn svn git-lfs";

pub(super) fn custom(section: &str, inventory: &[String]) -> bool {
    !section.is_empty()
        && section
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !RESERVED
            .split_ascii_whitespace()
            .any(|name| name == section)
        && !inventory.iter().any(|name| name == section)
}
