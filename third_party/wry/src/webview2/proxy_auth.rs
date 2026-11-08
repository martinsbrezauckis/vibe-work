//! Native proxy authentication installed before navigation by the WebView2 backend.
//! Credentials stay in the host process; only the exact configured proxy URI can receive them.

use crate::{ProxyEndpoint, Result};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::core::{Interface, HSTRING, PWSTR};

pub(super) fn install(webview: &ICoreWebView2, endpoint: &ProxyEndpoint) -> Result<()> {
  let Some((username, password)) = endpoint.credentials.clone() else {
    return Ok(());
  };
  let authority = format!("http://{}:{}", endpoint.host, endpoint.port);
  // WebView2 documents that Uri is the proxy URI for a proxy challenge, rather than the target URI.
  // Never answer an origin's challenge with these credentials, even when it copies our realm.
  unsafe {
    let webview = webview.cast::<ICoreWebView2_10>()?;
    let mut token = 0;
    webview.add_BasicAuthenticationRequested(
      &BasicAuthenticationRequestedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else {
          return Ok(());
        };
        let mut uri = PWSTR::null();
        args.Uri(&mut uri)?;
        let uri = take_pwstr(uri);
        if uri != authority && uri != format!("{authority}/") {
          return Ok(());
        }
        let response = args.Response()?;
        response.SetUserName(&HSTRING::from(&username))?;
        response.SetPassword(&HSTRING::from(&password))?;
        Ok(())
      })),
      &mut token,
    )?;
  }
  Ok(())
}
