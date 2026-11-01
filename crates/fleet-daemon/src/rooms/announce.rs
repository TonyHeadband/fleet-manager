//! The daemon's own announcements in a product room: a landed PR, a migration number taken. They wake nobody.
use super::posts::log_post;
use super::product_room;
use crate::store::{Store, StoreError};
use fleet_api::{MemberRef, Post, PostKind};

const SENDER: &str = "Fleet Manager";

/// "landed: <title> · <url>".
pub async fn landed(
    store: &Store,
    product: &str,
    task_title: &str,
    url: &str,
) -> Result<(), StoreError> {
    announce(store, product, format!("landed: {task_title} · {url}")).await
}

/// "migration <file> taken by <title> on <branch>", one announcement per file. The branch tells a later task of the
/// same title apart from a PR opened again.
pub async fn migrations_taken(
    store: &Store,
    product: &str,
    task_title: &str,
    branch: &str,
    files: &[String],
) -> Result<(), StoreError> {
    for f in files {
        announce(
            store,
            product,
            format!("migration {f} taken by {task_title} on {branch}"),
        )
        .await?;
    }
    Ok(())
}

/// Stores the text in the product's room unless the daemon said exactly it there already, so a re-detected merge or
/// a PR opened again for the same branch is announced once.
async fn announce(store: &Store, product: &str, text: String) -> Result<(), StoreError> {
    let room = product_room(store, product).await?;
    if store.daemon_said(&room, &text).await? {
        return Ok(());
    }
    let p = Post {
        id: 0,
        room,
        sender: MemberRef::Daemon,
        sender_name: SENDER.into(),
        to: None,
        kind: PostKind::Announcement,
        reply_to: None,
        text,
        at: crate::store::now_secs(),
        answered_at: None,
        breakdown_version: None,
    };
    store.add_post(&p).await?;
    log_post(&p);
    Ok(())
}
