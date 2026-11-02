//! Where the editions are released: the framework's repository, since sapphire-sync
//! moved into it (#394).

use sapphire_sync_server::Edition;

#[test]
fn both_editions_are_released_from_the_framework_repository() {
    for edition in [Edition::Headless, Edition::Desktop] {
        assert_eq!(
            edition.releases().repository(),
            "sapphire-turtle/sapphire-framework"
        );
    }
}
