//! H-owned consumer contract: import the common fixture from outside fusion_source.
//! This is not a test implementation of a foreign module or another installer.
use crate::fusion_source::service::FusionStorage;
use crate::fusion_source::test_support::{CreatorHarness, CONTENT, USER_DATA};

#[tokio::test]
async fn common_harness_is_reused_from_a_root_test_module() {
    let first = CreatorHarness::synthetic().await;
    let second = CreatorHarness::synthetic().await;
    assert_ne!(first.database, second.database);
    assert_ne!(first.library, second.library);
    assert_ne!(first.external_workspace, second.external_workspace);
    assert_ne!(first.request.user_files, second.request.user_files);
    assert!(!first.author.exists());
    assert!(!second.author.exists());

    let result = first
        .service
        .game_install(&first.game, first.request.clone())
        .await
        .unwrap();
    assert_eq!(result.status, "completed");
    assert!(
        second
            .storage
            .get_active_snapshot(&second.game)
            .unwrap()
            .is_none(),
        "a shared Source ID must not share SQLite state between consumers"
    );
    second
        .service
        .game_install(&second.game, second.request.clone())
        .await
        .unwrap();
    let a = first.snapshot();
    let b = second.snapshot();
    assert_ne!(a.workspace_root, b.workspace_root);
    assert!(a.workspace_root.starts_with(&first.external_workspace));
    assert!(b.workspace_root.starts_with(&second.external_workspace));
    for snapshot in [a, b] {
        assert_eq!(
            std::fs::read(snapshot.workspace_root.join("content/data/readme.txt")).unwrap(),
            CONTENT
        );
        assert_eq!(
            std::fs::read(snapshot.workspace_root.join("user/player.bin")).unwrap(),
            USER_DATA
        );
    }
}
