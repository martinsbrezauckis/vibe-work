//! Credential-free CLI entry point; shared with the app's shipped tools binary.
fn main() {
    match vw_gh_broker::entry::shim() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("gh broker: {error}");
            std::process::exit(1);
        }
    }
}
