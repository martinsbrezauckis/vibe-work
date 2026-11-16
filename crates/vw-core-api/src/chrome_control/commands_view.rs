//! The Settings browser card and the agent's view of Your Chrome: one read of the connection, setup and profile
//! state (`view`, `addon`). Read-only; called by commands, status publishing and the agent tools.
use super::commands::store_review;
use super::{ChromeControlView, PairingState, agent_browser::Browser, setup};
use vw_platform::sync::LockRecover;

pub(crate) fn view(shared: &crate::core::Shared) -> ChromeControlView {
    super::profile_presence::prune(shared);
    let chrome = &shared.chrome_control;
    let mut setup = setup::view(&chrome.config, chrome.dir(), &shared.instructions.own_home);
    setup.store_review = setup.state != setup::SetupState::Absent && store_review(shared);
    let pairing = chrome.owner.pairing.lock_or_recover();
    if setup.package
        && setup.state != setup::SetupState::RepairRequired
        && pairing.0 != PairingState::Unknown
    {
        setup.pairing = pairing.0.label("Chrome");
    }
    setup.pairing_state = pairing.0;
    if setup.state == setup::SetupState::WaitingForExtension
        && setup.pairing_state == PairingState::Paired
    {
        setup.state = setup::SetupState::ExtensionAdded;
    }
    if setup.package {
        setup.fingerprint.clone_from(&pairing.1);
    }
    let pairing_state = pairing.0;
    drop(pairing);
    if chrome.config.is_macos() {
        setup.profiles = super::installation::reported_profiles(shared, Browser::Chrome);
    }
    setup.profile_names_refused = super::browser_data::refused(chrome, Browser::Chrome);
    setup.addon = Some(addon_view(shared, &setup, pairing_state));
    let left = extension_left(chrome);
    let external = chrome
        .removed_external
        .load(std::sync::atomic::Ordering::SeqCst);
    setup.extension_left = (left && !external).then_some(true);
    setup.extension_restart = (left && external).then_some(true);
    let selections = chrome.selections.lock_or_recover();
    let edge = cfg!(feature = "your-edge").then(|| {
        let mut edge = setup::view_for(
            &chrome.config,
            chrome.dir(),
            &shared.instructions.own_home,
            "Microsoft Edge",
        );
        edge.store_review = setup.store_review;
        let pairing = chrome.owner.edge_pairing.lock_or_recover();
        // Like Chrome, a broken shared helper keeps its reinstall diagnosis ahead of pairing state.
        if edge.package && edge.state != setup::SetupState::RepairRequired {
            edge.pairing = if pairing.0 == PairingState::Unknown {
                vw_vocabulary::texts::EDGE_NOT_CONNECTED.into()
            } else {
                pairing.0.label("Edge")
            };
            edge.fingerprint.clone_from(&pairing.1);
        }
        edge.pairing_state = pairing.0;
        if edge.state == setup::SetupState::WaitingForExtension
            && edge.pairing_state == PairingState::Paired
        {
            edge.state = setup::SetupState::ExtensionAdded;
        }
        if let Some(reason) = super::edge_policy::refusal(&chrome.config) {
            edge.pairing = reason.into();
            edge.pairing_state = PairingState::Blocked;
        }
        drop(pairing);
        if chrome.config.is_macos() {
            edge.profiles = super::installation::reported_profiles(shared, Browser::Edge);
        }
        edge.profile_names_refused = super::browser_data::refused(chrome, Browser::Edge);
        edge.addon = Some(super::agent_browser::edge_addon(shared, &edge));
        Box::new(edge)
    });
    ChromeControlView {
        browser_preferences: Some(shared.browser_settings.view(shared.preview.is_some())),
        firefox: super::firefox_commands::setup_view(shared),
        safari: super::safari::view(shared),
        edge,
        pending_selections: selections.ids(),
        selection_error: selections.error.clone(),
        enabled: chrome.enabled(),
        edge_enabled: cfg!(feature = "your-edge").then(|| chrome.edge_enabled()),
        firefox_enabled: cfg!(feature = "your-firefox").then(|| chrome.firefox_enabled()),
        firefox_sharing: cfg!(feature = "your-firefox")
            .then(|| super::firefox_sharing::shown(shared))
            .flatten(),
        local_network: chrome.local_network(),
        local_network_applied: chrome.local_network_applied(),
        work: super::work::view(chrome),
        paired_profiles: super::profiles::optional_view(shared),
        chat_profiles: super::profile_selection::view(chrome),
        frame_colours: Some(super::frames::colours(shared))
            .filter(|colours| !colours.is_empty())
            .map(Box::new),
        setup,
    }
}

/// Your Chrome's state now (the card's and the agent's).
pub(crate) fn addon(shared: &crate::core::Shared) -> super::AddonView {
    view(shared)
        .setup
        .addon
        .unwrap_or_else(|| super::addon::view(super::AddonState::NotConnected, ""))
}

/// Your Chrome's one state (`addon`), from the live connection first and Chrome's files last.
fn addon_view(
    shared: &crate::core::Shared,
    setup: &setup::ChromeSetupView,
    pairing: PairingState,
) -> super::AddonView {
    if let Some(problem) = setup
        .addon
        .as_ref()
        .filter(|addon| addon.state == super::AddonState::AccessBlocked)
    {
        return problem.clone();
    }
    if setup.state == setup::SetupState::RepairRequired {
        let mut view = super::addon::view(super::AddonState::HelperSetupRequired, "");
        view.extension_version = super::installation::version(shared, Browser::Chrome);
        return view;
    }
    let chrome = &shared.chrome_control;
    let (waiting, connecting) = {
        let enroll = chrome.owner.enroll.lock_or_recover();
        (
            enroll.waiting_reported(),
            enroll.connecting(tokio::time::Instant::now()),
        )
    };
    let profiles = super::installation::profiles(shared, Browser::Chrome);
    let installed_version = super::addon::installed_version(
        &chrome.config,
        &profiles,
        &chrome.config.extension_id,
        pairing == PairingState::Paired,
        &chrome
            .owner
            .connected_profiles
            .snapshot()
            .into_iter()
            .filter(|(id, _)| Browser::Chrome.owns_profile(id))
            .collect::<Vec<_>>(),
    );
    let paired_once = chrome.paired_once.load(std::sync::atomic::Ordering::SeqCst);
    let removed = super::profiles::all_removed(chrome, Browser::Chrome);
    let facts = super::addon::Facts {
        package: setup.package,
        package_text: setup.pairing.clone(),
        set_up: setup.state != setup::SetupState::NotSetUp
            && setup.state != setup::SetupState::Absent,
        browser_issue: setup.browser_issue,
        enabled: chrome.enabled(),
        connected: pairing == PairingState::Paired,
        store_review: setup.store_review,
        other_owner: pairing == PairingState::OtherOwner,
        connecting: connecting || pairing == PairingState::Connecting,
        waiting,
        credential_missing: chrome
            .owner
            .credential_missing
            .load(std::sync::atomic::Ordering::SeqCst),
        keyring_unavailable: chrome
            .owner
            .keyring_unavailable
            .load(std::sync::atomic::Ordering::SeqCst),
        helper_up: !chrome.owner.tools.lock_or_recover().is_empty(),
        helper_conflict: chrome
            .owner
            .helper_conflict
            .load(std::sync::atomic::Ordering::SeqCst),
        paired_once,
        removed,
        // Only a disconnected remembered pairing needs an OS probe; live tools avoid a process per read.
        chrome_running: paired_once
            && pairing != PairingState::Paired
            && chrome.enabled()
            && chrome.config.launcher.is_none()
            && super::browser::running(&chrome.config),
        install_state: setup.install_state.clone(),
        extension_folder: !profiles.is_empty() || (chrome.config.is_macos() && paired_once),
        installed_version,
        min_version: shared
            .config
            .vibe_install
            .as_ref()
            .and_then(vw_vibe_install::VibeInstaller::extension_min_version),
    };
    let state = super::addon::state(&facts, || {
        chrome.prober.probe_for(
            &chrome.config,
            profiles.clone(),
            &chrome.config.extension_id,
            std::time::Instant::now(),
        )
    });
    let state = super::installation::observed_state(&chrome.config, state);
    let mut view = super::addon::view(state, &facts.package_text);
    view.extension_version = super::installation::version(shared, Browser::Chrome);
    super::addon::reconnecting(&mut view, paired_once && !waiting && !removed);
    super::profile_reason::window_card(chrome, &mut view, Browser::Chrome);
    super::installation::present(&chrome.config, &mut view);
    view
}

/// After Remove: the extension is still in a profile of the folders Remove recorded.
fn extension_left(chrome: &super::ChromeControl) -> bool {
    chrome.left_in_chrome.lock_or_recover().iter().any(|dir| {
        !super::probe::profiles_for(&chrome.config, dir, &chrome.config.extension_id).is_empty()
    })
}
