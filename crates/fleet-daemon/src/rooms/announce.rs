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

/// How the product room names a task: its title, or its name when it has none or an empty one.
pub(crate) fn task_label(title: Option<&str>, name: &str) -> String {
    title.filter(|t| !t.is_empty()).unwrap_or(name).to_string()
}

/// "<title> · <notice>", one announcement per event: unlike the others it is not deduplicated, as the same notice
/// ("checks passed") is a new event each time it happens.
pub async fn task_notice(
    store: &Store,
    product: &str,
    task_title: &str,
    notice: &str,
) -> Result<(), StoreError> {
    let room = product_room(store, product).await?;
    post(store, room, format!("{task_title} · {notice}")).await
}

/// Stores the text in the product's room unless the daemon said exactly it there already, so a re-detected merge or
/// a PR opened again for the same branch is announced once.
async fn announce(store: &Store, product: &str, text: String) -> Result<(), StoreError> {
    let room = product_room(store, product).await?;
    if store.daemon_said(&room, &text).await? {
        return Ok(());
    }
    post(store, room, text).await
}

async fn post(store: &Store, room: String, text: String) -> Result<(), StoreError> {
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
