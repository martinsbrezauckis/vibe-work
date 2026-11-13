//! Local navigation checks the live extension toggle before dispatch, in the same Ask browsing tier.
use serde_json::json;
#[tokio::test]
async fn local_navigation_needs_a_confirmed_file_access_toggle() {
    for granted in [false, true] {
        let core = super::read_consent_tests::core(&format!("local-file-toggle-{granted}")).await;
        let folder = std::path::PathBuf::from(
            std::env::var_os("VW_TEST_ROOT")
                .unwrap_or_else(|| std::env::temp_dir().into_os_string()),
        )
        .join(format!(
            "local-toggle-chat-{}-{granted}",
            std::process::id()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        // Not resolved: on macOS the chat folder is named under the /var -> /private/var link, as the person sees it.
        let path = folder.join("local.html");
        std::fs::write(&path, "<h1>Local</h1>").unwrap();
        let _ = core
            .shared
            .store
            .load(
                json!({"session":{"id":"s","cwd":folder},"history":[],"turns":[]}),
                vw_session::LoadKind::New,
            )
            .unwrap();
        let seen = super::test_support::answering(&core.shared, move |tool, args| match tool {
            "chrome_status" => {
                json!({"structuredContent":{"profiles":[{"profileId":"chrome:p","connected":true,"extensionCapabilities":["transport.contract.v1"]}],"sessions":[{"sessionId":"t","profileId":"chrome:p"}]}})
            }
            "owner.primitive" => {
                assert_eq!(args["steps"][0]["method"], "permissions.status");
                json!({"status":"completed","data":{"results":{"state":{"fileAccess":granted}}}})
            }
            _ => panic!("unexpected {tool}"),
        });
        let mut request = json!({"sessionId":"t","action":"page.navigate","args":{"tabId":7,"url":url::Url::from_file_path(path).unwrap().as_str()}});
        let result = super::network::prepare(&core.shared, "s", &mut request).await;
        if granted {
            assert!(result.is_ok(), "{result:?}");
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("Allow access to file URLs"), "{error}");
        }
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|(name, _)| name == "chrome_execute")
        );
        core.shutdown().await;
        std::fs::remove_dir_all(folder).unwrap();
    }
}
