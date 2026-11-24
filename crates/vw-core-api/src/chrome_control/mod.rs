//! App-owned Chrome connection. The chat access mode owns permissions.
//! Used by agent tools and Settings; pairing is independent of action authority.
mod action_tracking;
mod addon;
#[cfg(test)]
mod agent_profile_text_tests;
#[cfg(test)]
mod profile_default_refusal_tests;
mod safari;
mod safari_copies;
mod safari_package;
mod safari_pairing;
#[cfg(any(all(feature = "your-safari", target_os = "macos"), test))]
mod safari_startup;
pub use safari::SafariView;
mod agent_browser;
mod agent_close;
mod agent_windows;
mod approval;
#[cfg(all(test, feature = "your-edge"))]
mod approval_tests;
mod availability;
mod batch;
mod batch_wait;
mod browser;
mod browser_start;
#[cfg(all(test, feature = "your-edge"))]
mod browser_start_tests;
mod browser_text;
mod builtin;
#[cfg(test)]
mod builtin_send_tests;
mod builtin_share;
mod capabilities;
#[cfg(test)]
mod capability_tests;
mod check;
mod commands;
mod commands_view;
mod connection_notice;
mod content;
mod deadline;
#[cfg(test)]
#[cfg(feature = "your-edge")]
mod edge_agent_tests;
#[cfg(test)]
#[cfg(feature = "your-edge")]
mod edge_lifecycle_tests;
mod edge_policy;
mod edge_state;
mod enroll;
mod enroll_browser;
mod escalation;
mod extension_install;
mod external_video;
#[cfg(test)]
mod external_video_adversarial_tests;
mod external_video_file;
#[cfg(test)]
mod external_video_isolation_tests;
#[cfg(test)]
mod external_video_lifecycle_tests;
#[cfg(test)]
mod external_video_ownership_tests;
#[cfg(test)]
mod external_video_race_tests;
#[cfg(test)]
mod no_keeps_tabs_tests;
use vw_platform::extension_registry;
#[cfg(test)]
#[cfg(feature = "your-firefox")]
mod firefox_boundary_tests;
mod firefox_commands;
mod firefox_package;
mod firefox_probe;
mod firefox_setup;
mod firefox_sharing;
#[cfg(all(test, feature = "your-firefox"))]
mod firefox_sharing_tests;
#[cfg(all(test, feature = "your-firefox", feature = "your-edge"))]
mod firefox_switch_tests;
mod frame_colours;
mod frames;
mod gateway;
mod helper_refresh;
#[cfg(all(test, feature = "your-edge"))]
mod inactive_profile_tests;
mod lifecycle;
#[cfg(test)]
mod live_tests;
use lifecycle::{refresh_helper_held, start_owner_for};
pub(crate) use lifecycle::{start_owner, startup, stop_owner};
mod local_documents;
mod login;
mod mapping;
mod network;
#[cfg(test)]
mod observation_output_tests;
mod observation_support;
#[cfg(test)]
mod observation_support_tests;
#[cfg(test)]
mod observation_tests;
mod output;
mod output_batch;
mod output_cache;
mod output_file;
mod output_fragment;
#[cfg(test)]
mod output_guard_tests;
mod output_receipt;
mod output_status;
#[cfg(test)]
mod output_tests;
#[cfg(test)]
mod output_walk_tests;
mod owner;
#[cfg(test)]
mod owner_credential_tests;
#[cfg(test)]
mod owner_keyring_tests;
#[cfg(test)]
mod page_tool_output_tests;
mod pairing;
mod pairing_approval;
#[cfg(test)]
mod status_output_tests;
#[cfg(test)]
mod status_session_budget_tests;
pub use pairing_approval::BROWSER_PAIRING_DIALOG_METHOD;
#[cfg(test)]
mod pairing_approval_profile_tests;
#[cfg(test)]
mod pairing_approval_tests;
#[cfg(test)]
mod pairing_key_change_tests;
#[cfg(test)]
mod pairing_removed_connect_tests;
#[cfg(test)]
mod pairing_removed_setup_tests;
#[cfg(test)]
mod pairing_removed_tests;
pub use pairing::PairingState;
#[cfg(test)]
mod admission_order_tests;
#[cfg(test)]
mod ask_policy_tests;
#[cfg(test)]
mod close_consent_tests;
#[cfg(test)]
mod held_stop_tests;
mod interaction_consent;
#[cfg(test)]
mod interaction_consent_tests;
mod person_stop;
#[cfg(test)]
mod person_stop_tests;
#[cfg(test)]
mod pill_stop_tests;
mod pipe;
mod probe;
mod profile_agent;
#[cfg(test)]
mod profile_alias_tests;
mod profile_bindings;
mod profile_commands;
mod profile_seeding;
mod profile_selection;
#[cfg(test)]
mod stop_turn_boundary_tests;
pub use profile_selection::ChatBrowserProfile;
mod profiles;
pub use profiles::{BrowserProfile, ProfileState};
mod browser_data;
#[cfg(test)]
mod profile_card_agent_tests;
#[cfg(test)]
mod profile_choice_offer_tests;
#[cfg(test)]
mod profile_choice_tests;
mod profile_folders;
mod profile_ids;
mod profile_names;
#[cfg(all(test, feature = "your-edge"))]
mod profile_names_access_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_names_label_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_names_macos_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_names_tests;
mod profile_presence;
#[cfg(all(test, feature = "your-edge"))]
mod profile_presence_tests;
mod profile_reason;
#[cfg(all(test, feature = "your-edge"))]
mod profile_reinstall_tests;
#[cfg(test)]
mod profile_retire_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_scope_tests;
mod profile_storage;
mod read_consent;
#[cfg(test)]
mod read_consent_tests;
#[cfg(test)]
mod read_export_tests;
#[cfg(test)]
mod read_output_tests;

#[cfg(test)]
mod output_secret_tests;
mod receipt;
#[cfg(test)]
mod receipt_tests;
mod reconnect;
#[cfg(all(test, feature = "your-edge"))]
mod reconnect_open_tests;
#[cfg(all(test, feature = "your-edge"))]
mod reconnect_tests;
#[cfg(test)]
mod registry_native_tests;
mod remove;
#[cfg(all(test, feature = "your-edge"))]
mod remove_save_tests;
#[cfg(all(test, feature = "your-safari"))]
mod safari_action_tests;
#[cfg(all(test, feature = "your-safari"))]
mod safari_pairing_tests;
#[cfg(all(test, feature = "your-safari"))]
mod safari_supersede_tests;
#[cfg(all(test, feature = "your-safari"))]
mod safari_tests;
#[cfg(test)]
mod secret_policy_tests;
#[cfg(test)]
mod security_tests;
mod selection;
mod session_activity;
#[cfg(test)]
mod session_lifetime_tests;
#[cfg(test)]
mod session_modal_tests;
#[cfg(test)]
mod session_resume_tests;
#[cfg(all(test, feature = "your-edge"))]
mod session_routing_tests;
mod session_start;
mod session_title;
mod session_title_output;
#[cfg(test)]
mod session_title_output_tests;
mod setup;
mod store;
mod tabs;
pub use tabs::ChromeTab;
#[cfg(test)]
mod tab_pick_tests;
/// Test-only drivers for core tests that exercise browser control (never compiled into the app).
#[cfg(test)]
pub(crate) mod test_support;
mod work;
pub use addon::{AddonState, AddonView};
pub use commands::{ChromeControlRequest, ChromeControlView};
pub use selection::{ChromeSelection, ChromeShareKind};
mod sharing;
pub use setup::{ChromeControlConfig, ChromeSetupView};
pub use work::ChromeWorkView;
mod state;
pub(crate) use state::ChromeControl;

// The facade. Every submodule above is private: the rest of the core reaches browser control only through the
// items below (and the `pub use` items `lib.rs` re-exports), so this block is the whole interface.
/// Agent tool gateway: the `chrome_*` MCP tools, their browser choice and the batch/login/consent preparation.
pub(crate) use agent_browser::{Browser, Fixed as FixedText, selected as selected_browser};
pub(crate) use agent_close::prepare as prepare_close;
pub(crate) use approval::project as project_approvals;
pub(crate) use batch::{prepare as prepare_batch, steps as batch_steps};
#[cfg(test)]
pub(crate) use builtin::required as builtin_required;
/// The built-in browser's action policy (sending, edits, approval tiers, packages).
pub(crate) use builtin::{
    SEND_ACTION as BUILTIN_SEND_ACTION, accepts_edits as builtin_accepts_edits,
    activates_send as builtin_activates_send, approval_tier as builtin_approval_tier,
    ask_required as builtin_ask_required, check_full as builtin_check_full,
    explain_form_send as builtin_explain_form_send, may_send as builtin_may_send,
    package_actions as builtin_package_actions,
};
pub(crate) use builtin_share::queue as queue_builtin_share;
/// The chat mode's authority over browser actions (agent tools, ask cards, model and session changes).
pub(crate) use check::{
    check as check_request, check_full, mode as chat_mode, mode_changed, mode_refusal,
};
#[cfg(test)]
pub(crate) use commands::addon as addon_view;
/// Settings, session facts and saved page files.
pub(crate) use commands::{extension_store, view as settings_view};
pub(crate) use frames::sync_chat as sync_frames;
pub(crate) use gateway::{call as call_tool, is_tool, list as list_tools, validate_execute_args};
pub(crate) use installation::addons;
pub(crate) use interaction_consent::{
    checks as interaction_checked, prepare as prepare_interaction,
};
pub(crate) use login::prepare as prepare_login;
pub(crate) use page_files::save as save_page_file;
pub(crate) use read_consent::{
    needed as read_consent_needed, personal_read, prepare as prepare_read,
};
pub(crate) use safari::enabled as safari_enabled;

/// Before a `chrome_session` open: the person picks this chat's profile on a card when it is not decided
/// (`profile_seeding::ask_person`). The open's own arguments are unchanged; it then runs in the chat's binding.
pub(crate) async fn ask_profile(
    shared: &std::sync::Arc<crate::core::Shared>,
    chat: &str,
    args: &serde_json::Value,
) -> Result<(), String> {
    profile_seeding::ask_person(shared, chat, &mut args.clone()).await
}
/// Exact browser identity comparison, including the legacy raw Chrome UUID alias.
pub(crate) fn same_profile(left: &str, right: &str) -> bool {
    profile_ids::same(left, right)
}
pub(crate) fn end_chat(shared: &crate::core::Shared, chat: &str, deleted: bool) {
    shared.chrome_control.end_chat(chat, deleted);
    if deleted {
        frames::forget_chat(shared, chat);
    }
    if deleted {
        profile_presence::prune(shared);
    }
    owner::publish(shared);
}
/// The person stopped the chat in Vibe Work: its Chrome work ends and is not reopened by an earlier id.
pub(crate) fn stop_chat(shared: &crate::core::Shared, chat: &str) {
    shared.chrome_control.stop_chat(chat);
    frames::sync_chat(shared, chat);
    owner::publish(shared);
}
/// Publish availability after the question and all browsers' released state are visible.
pub(crate) fn publish_handover(shared: &crate::core::Shared) {
    owner::publish(shared);
}
/// A completed turn releases visual control and retains the session for follow-up.
pub(crate) fn end_turn(shared: &crate::core::Shared, chat: &str) {
    if shared.asks.question_hold(chat).is_some() {
        shared.chrome_control.handover(chat);
    } else {
        shared.chrome_control.end_turn(chat);
    }
    // The agent lets go visually at turn end; the session stays for a follow-up.
    frames::sync_chat(shared, chat);
    owner::publish(shared);
}
#[cfg(feature = "ts")]
mod declarations;
#[cfg(feature = "ts")]
pub(crate) use declarations::declarations;

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod pairing_tests;

#[cfg(test)]
mod connected_schema_tests;

mod connected_schema;

mod installation;

#[cfg(all(test, feature = "your-edge"))]
mod browser_addon_tests;

#[cfg(test)]
mod status_actions_tests;

mod status_actions;

#[cfg(test)]
mod schema_budget_tests;
#[cfg(test)]
mod summary_support_tests;

#[cfg(all(test, feature = "your-edge"))]
mod browser_switch_tests;

mod switches;

#[cfg(any(target_os = "macos", all(test, unix)))]
mod safari_registration;

#[cfg(test)]
mod agent_outcome_tests;
#[cfg(test)]
mod sharing_tests;

#[cfg(test)]
mod page_media_tests;
#[cfg(test)]
mod page_video_tests;

#[cfg(test)]
mod local_file_tests;
mod page_capture;
mod page_files;
mod page_plan;
mod page_plan_catalog;
mod page_resources;
#[cfg(feature = "your-safari")]
mod safari_capture;

#[cfg(test)]
mod extension_state_tests;

mod extension_state;

#[cfg(test)]
mod page_capture_tests;

#[cfg(test)]
mod data_export_tests;

#[cfg(all(test, feature = "your-firefox", feature = "your-edge"))]
mod agent_close_dbk17u_tests;
#[cfg(all(test, feature = "your-firefox", feature = "your-edge"))]
mod agent_close_incarnation_tests;
#[cfg(all(test, feature = "your-firefox", feature = "your-edge"))]
mod agent_close_restart_tests;
#[cfg(all(test, feature = "your-firefox", feature = "your-edge"))]
mod agent_close_window_tests;
#[cfg(all(test, feature = "your-firefox", feature = "your-edge"))]
mod agent_firefox_close_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_binding_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_rebind_tests;

#[cfg(all(test, feature = "your-edge"))]
mod profile_agent_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_history_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_off_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_selection_tests;
#[cfg(all(test, feature = "your-edge"))]
mod profile_settings_tests;

#[cfg(feature = "your-safari")]
mod safari_service;

#[cfg(test)]
mod browser_availability_text_tests;
