use crate::{
    api::{string, Mutation, Transport},
    forms::{now, open, settings},
    model::{workspace_url, Kind, Listing},
    view::{self, ICON},
};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tezbar_extension_sdk::{no_view, CoreResult, ExtensionContext};
use tokio::sync::Mutex;

const RESOURCE: &str = "linear-inbox";
const PERIOD: Duration = Duration::from_secs(15 * 60);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
fn callback_id(action: &str, generation: u64, revision: u64) -> String {
    format!("{action}:{generation}:{revision}")
}
fn validate_callback<'a>(
    action: &'a str,
    generation: u64,
    revision: u64,
) -> Result<&'a str, String> {
    let (rest, actual_revision) = action
        .rsplit_once(':')
        .ok_or("Unknown Linear tray action.")?;
    let (verb, actual_generation) = rest.rsplit_once(':').ok_or("Unknown Linear tray action.")?;
    if actual_revision.parse::<u64>().ok() != Some(revision)
        || actual_generation.parse::<u64>().ok() != Some(generation)
    {
        return Err("The inbox menu changed. Reopen it to see the latest notifications.".into());
    }
    Ok(verb)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callbacks_cannot_cross_menu_restart_or_refresh() {
        for action in [
            "read-0",
            "open-0",
            "read-all",
            "disable",
            "refresh",
            "open-inbox",
            "configure",
        ] {
            let old = callback_id(action, 1, 1);
            assert_eq!(validate_callback(&old, 1, 1).unwrap(), action);
            assert!(validate_callback(&old, 2, 1).is_err());
            assert!(validate_callback(&old, 1, 2).is_err());
            assert!(validate_callback(action, 1, 1).is_err());
        }
    }
}

#[derive(Clone)]
pub struct Tray {
    transport: Transport,
    inner: Arc<Mutex<Option<Inbox>>>,
}

struct Inbox {
    generation: u64,
    revision: u64,
    key: String,
    grouped: bool,
    always_show: bool,
    listing: Listing,
}

impl Tray {
    pub fn new(transport: Transport) -> Self {
        Self {
            transport,
            inner: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn start(
        &self,
        ctx: &ExtensionContext,
        background: &ExtensionContext,
        prefs: &Value,
    ) -> CoreResult {
        let mut state = self.inner.lock().await;
        let generation = state
            .as_ref()
            .map(|old| old.generation)
            .unwrap_or_else(|| NEXT_GENERATION.fetch_add(1, Ordering::Relaxed));
        let mut next = Inbox {
            generation,
            revision: state.as_ref().map(|old| old.revision).unwrap_or(0),
            key: string(prefs, "linearApiKey").trim().into(),
            grouped: prefs["groupNotifications"] == true,
            always_show: prefs["alwaysShow"] != false,
            listing: Listing::new(Kind::Unread, String::new()),
        };
        self.refresh(&mut next).await?;
        let start = state.is_none();
        if start {
            ctx.host(
                "host.background.retain",
                json!({"key":RESOURCE,"durable":false}),
            )
            .await?;
        }
        if let Err(error) = self.publish(ctx, &mut next).await {
            if start {
                let _ = ctx
                    .host("host.background.release", json!({"key":RESOURCE}))
                    .await;
            }
            return Err(error);
        }
        if start {
            let client = self.clone();
            let worker = background.clone();
            if let Err(error)=background.spawn(async move {
                loop {
                    tokio::time::sleep(PERIOD).await;
                    let mut guard=client.inner.lock().await;
                    let Some(inbox)=guard.as_mut().filter(|inbox|inbox.generation==generation) else { break };
                    if client.refresh(inbox).await.is_ok() {
                        let _=client.publish(&worker,inbox).await;
                    } else {
                        inbox.revision = inbox.revision.wrapping_add(1);
                        let _=worker.host("host.tray.set",json!({
                            "resourceId":RESOURCE,"title":"Linear","tooltip":"Linear inbox couldn't refresh",
                            "icon":ICON,"items":[
                                {"id":callback_id("refresh",inbox.generation,inbox.revision),"title":"Refresh"},
                                {"id":callback_id("configure",inbox.generation,inbox.revision),"title":"Configure Linear"},
                                {"id":callback_id("disable",inbox.generation,inbox.revision),"title":"Stop Inbox Menu"}
                            ],
                        })).await;
                    }
                }
            }) {
                let _=ctx.host("host.tray.remove",json!({"resourceId":RESOURCE})).await;
                let _=ctx.host("host.background.release",json!({"key":RESOURCE})).await;
                return Err(error);
            }
        }
        *state = Some(next);
        Ok(no_view("Linear inbox menu enabled"))
    }

    async fn refresh(&self, inbox: &mut Inbox) -> Result<(), String> {
        let api = self.transport.authenticate(&inbox.key)?;
        let prefs = json!({"groupNotifications":inbox.grouped});
        let mut listing = Listing::new(Kind::Unread, String::new());
        tokio::time::timeout(Duration::from_secs(23), async {
            listing.load(&api, &prefs, false).await?;
            while listing.cursor.is_some() {
                listing.load(&api, &prefs, true).await?;
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(|_| "Linear inbox refresh timed out.")??;
        listing.rows.retain(view::unread);
        inbox.listing = listing;
        Ok(())
    }

    async fn publish(&self, ctx: &ExtensionContext, inbox: &mut Inbox) -> Result<(), String> {
        let count = inbox.listing.rows.len();
        let revision = inbox.revision.wrapping_add(1);
        if !inbox.always_show && count == 0 {
            ctx.host("host.tray.remove", json!({"resourceId":RESOURCE}))
                .await?;
            inbox.revision = revision;
            return Ok(());
        }
        let mut items = vec![
            json!({"id":callback_id("open-inbox",inbox.generation,revision),"title":"Open Inbox"}),
        ];
        if count > 0 {
            items.push(json!({"id":callback_id("read-all",inbox.generation,revision),"title":"Mark All as Read"}));
        }
        items.push(json!({"id":"unread-section","kind":"section","title":if count==0 {"No Unread Notifications"} else {"Unread Notifications"}}));
        for (i, row) in inbox.listing.rows.iter().take(35).enumerate() {
            let title = view::join(&[string(row, "subtitle"), string(row, "title")])
                .chars()
                .take(80)
                .collect::<String>();
            items.push(
                json!({"id":format!("note-{i}"),"kind":"submenu","title":title,"items":[
                    {"id":callback_id(&format!("open-{i}"),inbox.generation,revision),"title":"Open and Mark as Read"},
                    {"id":callback_id(&format!("read-{i}"),inbox.generation,revision),"title":"Mark as Read"},
                ]}),
            );
        }
        items.extend([
            json!({"id":"all-notifications","title":"View All Notifications","commandName":"notifications"}),
            json!({"id":callback_id("refresh",inbox.generation,revision),"title":"Refresh"}),
            json!({"id":callback_id("configure",inbox.generation,revision),"title":"Configure Linear"}),
            json!({"id":callback_id("disable",inbox.generation,revision),"title":"Stop Inbox Menu"}),
        ]);
        let title = if count == 0 {
            "Linear".into()
        } else {
            format!("Linear {count}")
        };
        ctx.host(
            "host.tray.set",
            json!({"resourceId":RESOURCE,"title":title,
            "tooltip":format!("Linear inbox · {count} unread"),"icon":ICON,"items":items}),
        )
        .await?;
        inbox.revision = revision;
        Ok(())
    }

    pub async fn event(&self, ctx: &ExtensionContext, payload: &Value) -> CoreResult {
        if payload["resourceId"] != RESOURCE {
            return Err("Unknown Linear tray resource.".into());
        }
        let action = payload["actionId"].as_str().unwrap_or_default();
        let mut state = self.inner.lock().await;
        let inbox = state.as_mut().ok_or("Linear inbox menu is disabled.")?;
        let action = validate_callback(action, inbox.generation, inbox.revision)?;
        match action {
            "open-inbox" => open(ctx, &workspace_url(&inbox.listing.organization, "inbox")).await?,
            "configure" => settings(ctx).await?,
            "disable" => {
                ctx.host("host.tray.remove", json!({"resourceId":RESOURCE}))
                    .await?;
                ctx.host("host.background.release", json!({"key":RESOURCE}))
                    .await?;
                *state = None;
            }
            "refresh" => {
                self.refresh(inbox).await?;
                self.publish(ctx, inbox).await?;
            }
            "read-all" => {
                let (api, grouped) = (self.transport.authenticate(&inbox.key)?, inbox.grouped);
                let stamp = now(0);
                while let Some(row) = inbox.listing.rows.first() {
                    let id = string(row, "id");
                    let result = read_notification(&api, &id, grouped, stamp.clone()).await;
                    if let Err(error) = result {
                        let _ = self.publish(ctx, inbox).await;
                        return Err(error);
                    }
                    inbox.listing.rows.remove(0);
                }
                self.publish(ctx, inbox).await?;
            }
            _ => {
                let (verb, index) = action
                    .split_once('-')
                    .ok_or("Unknown Linear tray action.")?;
                let index: usize = index.parse().map_err(|_| "Unknown Linear tray action.")?;
                let row = inbox
                    .listing
                    .rows
                    .get(index)
                    .ok_or("Notification is no longer in the inbox.")?;
                if verb == "open" {
                    let target = notification_url(row, &inbox.listing.organization);
                    open(ctx, &target).await?;
                } else if verb != "read" {
                    return Err("Unknown Linear tray action.".into());
                }
                let api = self.transport.authenticate(&inbox.key)?;
                read_notification(&api, &string(row, "id"), inbox.grouped, now(0)).await?;
                inbox.listing.rows.remove(index);
                self.publish(ctx, inbox).await?;
            }
        }
        Ok(json!({"ok":true}))
    }
}

pub fn notification_url(row: &Value, organization: &str) -> String {
    for field in [
        "url",
        "comment.url",
        "projectUpdate.url",
        "issue.url",
        "project.url",
    ] {
        let value = field.split('.').fold(row, |value, segment| &value[segment]);
        if let Some(value) = value.as_str().filter(|value| !value.is_empty()) {
            let full = if value.starts_with('/') && !value.starts_with("//") {
                format!("https://linear.app{value}")
            } else {
                value.into()
            };
            if super::forms::web_url(&full).is_ok() {
                return full;
            }
        }
    }
    workspace_url(organization, "inbox")
}

pub async fn read_notification(
    api: &crate::api::Api<'_>,
    id: &str,
    grouped: bool,
    stamp: Value,
) -> Result<(), String> {
    if grouped {
        let data=api.request("mutation ReadInboxGroup($id: String!) { inboxNotificationUpdate(id: $id, input: { read: true }) { success } }",json!({"id":id})).await?;
        if data["inboxNotificationUpdate"]["success"] != true {
            return Err("Linear did not confirm the notification change.".into());
        }
    } else {
        api.mutation(
            Mutation::NotificationUpdate,
            Some(id),
            Some(json!({"readAt":stamp})),
        )
        .await?;
    }
    Ok(())
}
