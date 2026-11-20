mod api;
mod forms;
mod model;
mod tray;
mod view;

use api::{string, Api, Mutation, Transport};
use model::{
    workspace_url, Action, Choice, Entity, Form, FormKind, Kind, Listing, NoteAction, Screen,
    Session,
};
use serde_json::{json, Value};
use std::sync::OnceLock;
use tezbar_extension_sdk::{
    serve, unchanged, BoxFuture, CommandConcurrency, CoreResult, Extension, ExtensionContext,
};

const COMMANDS: &[&str] = &[
    "create-issue",
    "search-issues",
    "assigned-issues",
    "created-issues",
    "active-cycle",
    "search-projects",
    "search-custom-views",
    "search-documents",
    "create-project",
    "notifications",
    "unread-notifications",
    "create-issue-for-myself",
    "quick-add-comment-to-issue",
    "favorites",
];

struct Linear {
    transport: Transport,
    tray: tray::Tray,
    background: OnceLock<ExtensionContext>,
}

impl Linear {
    async fn run(
        &self,
        ctx: &ExtensionContext,
        command: &str,
        payload: &Value,
        state: &mut Value,
    ) -> CoreResult {
        let prefs = &payload["preferences"];
        let key = prefs["linearApiKey"].as_str().unwrap_or_default().trim();
        let api = match self.transport.authenticate(key) {
            Ok(api) => api,
            Err(error)
                if !matches!(
                    command,
                    "unread-notifications"
                        | "create-issue-for-myself"
                        | "quick-add-comment-to-issue"
                ) =>
            {
                return Ok(view::auth_required(&error))
            }
            Err(error) => return Err(error),
        };
        let arguments = &payload["argumentValues"];
        match command {
            "create-issue-for-myself" => {
                return forms::quick_issue(&api, ctx, prefs, arguments).await
            }
            "quick-add-comment-to-issue" => {
                return forms::quick_comment(&api, ctx, prefs, arguments).await
            }
            "unread-notifications" => {
                return self
                    .tray
                    .start(
                        ctx,
                        self.background
                            .get()
                            .ok_or("Linear host is not initialized.")?,
                        prefs,
                    )
                    .await
            }
            _ => {}
        }
        let form = match command {
            "create-issue" => Some(FormKind::Issue),
            "create-project" => Some(FormKind::Project),
            _ => None,
        };
        let mut session = if let Some(kind) = form {
            let mut form = Form::new(kind, Value::Null, json!({}));
            form.load(&api, prefs).await?;
            Session::new(Screen::Form(form))
        } else {
            let kind = match command {
                "search-issues" => Kind::SearchIssues,
                "assigned-issues" => Kind::Assigned,
                "created-issues" => Kind::Created,
                "active-cycle" => Kind::Cycle,
                "search-projects" => Kind::Projects,
                "search-custom-views" => Kind::Views,
                "search-documents" => Kind::Documents,
                "notifications" => Kind::Notifications,
                "favorites" => Kind::Favorites,
                _ => return Err("Unknown Linear command.".into()),
            };
            let mut list = Listing::new(kind, String::new());
            if kind.remote_search() {
                list.query = search_text(payload);
            }
            let mut teams = Vec::new();
            match kind {
                Kind::Assigned => {
                    list.options = issue_scopes();
                    list.filter = "assigned".into();
                }
                Kind::Cycle | Kind::Projects => {
                    teams = model::teams(&api).await?;
                    list.options = model::choices(&teams, "name");
                    if kind == Kind::Projects {
                        list.options.insert(0, Choice::new("", "All Teams"));
                    } else {
                        let initial = teams
                            .iter()
                            .find(|team| team["activeCycle"]["id"].is_string())
                            .or_else(|| teams.first());
                        if let Some(team) = initial {
                            list.filter = string(team, "id");
                            list.target = string(&team["activeCycle"], "id");
                        }
                    }
                }
                Kind::Notifications => {
                    list.options = vec![
                        Choice::new("all", "All"),
                        Choice::new("unread", "Unread"),
                        Choice::new("snoozed", "Snoozed"),
                    ];
                    list.filter = "all".into();
                }
                _ => {}
            }
            let mut session = Session::new(Screen::List(list));
            session.cycle_teams = teams;
            if let Screen::List(list) = &mut session.screen {
                list.load(&api, prefs, false).await?;
            }
            session
        };
        finish(&mut session, prefs, state)
    }
}

fn finish(session: &mut Session, prefs: &Value, state: &mut Value) -> CoreResult {
    let view = view::render(session, prefs);
    *state = serde_json::to_value(session).map_err(|_| "Unable to save Linear session.")?;
    Ok(view)
}

fn issue_scopes() -> Vec<Choice> {
    vec![
        Choice::new("assigned", "Assigned"),
        Choice::new("created", "Created"),
        Choice::new("subscribed", "Subscribed"),
    ]
}

fn search_text(payload: &Value) -> String {
    payload["searchText"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(512)
        .collect()
}

async fn refresh(api: &Api<'_>, prefs: &Value, session: &mut Session) {
    let Screen::List(current) = &mut session.screen else {
        return;
    };
    let mut updated = current.clone();
    match updated.load(api, prefs, false).await {
        Ok(()) => *current = updated,
        Err(_) => session.notice = "The change succeeded. Refresh to update this list.".into(),
    }
}

async fn item(api: &Api<'_>, kind: Entity, entity: &Value) -> Result<Value, String> {
    let id = forms::required(entity, "id", "Item ID", 256)?;
    match kind {
        Entity::Issue => {
            let fields = format!("{} description labelIds", api::ISSUE);
            api.entity("Issue", "issue", &id, &fields).await
        }
        Entity::Project => api.entity("Project", "project", &id, api::PROJECT).await,
        Entity::Document => {
            let query=format!("query Document($id: ID!) {{ documents(first: 1, filter: {{ id: {{ eq: $id }} }}) {{ nodes {{ {} content }} }} }}",api::DOCUMENT);
            let data = api.request(&query, json!({"id":id})).await?;
            data["documents"]["nodes"]
                .as_array()
                .and_then(|nodes| nodes.first())
                .cloned()
                .ok_or("Linear could not find this document.".into())
        }
        _ => Ok(entity.clone()),
    }
}

async fn perform(
    api: &Api<'_>,
    ctx: &ExtensionContext,
    prefs: &Value,
    session: &mut Session,
    action: Action,
    params: &Value,
) -> Result<(), String> {
    match action {
        Action::Settings => forms::settings(ctx).await?,
        Action::Open(target) => forms::open(ctx, &target).await?,
        Action::Copy(text) => {
            tezbar_extension_sdk::copy(ctx, &text).await?;
        }
        Action::Refresh | Action::More => {
            if let Screen::List(list) = &mut session.screen {
                let mut updated = list.clone();
                updated
                    .load(api, prefs, matches!(action, Action::More))
                    .await?;
                *list = updated;
            }
        }
        Action::Back => {
            if let Some(screen) = session.history.pop() {
                session.screen = screen;
                if matches!(session.screen, Screen::List(_)) {
                    refresh(api, prefs, session).await;
                }
                if let Screen::Detail(entity, current) = &mut session.screen {
                    if let Ok(updated) = item(api, *entity, current).await {
                        *current = updated;
                    }
                }
            } else if let Screen::Result(entity, _, _) = session.screen {
                let kind = if entity == Entity::Project {
                    FormKind::Project
                } else {
                    FormKind::Issue
                };
                let mut form = Form::new(kind, Value::Null, json!({}));
                form.load(api, prefs).await?;
                session.screen = Screen::Form(form);
            }
        }
        Action::Select => {
            let selected = params["formValues"]["value"]
                .as_str()
                .ok_or("Select a Linear filter.")?;
            let Screen::List(list) = &mut session.screen else {
                return Err("No Linear list to filter.".into());
            };
            if !list.options.iter().any(|option| option.id == selected) {
                return Err("Select a valid filter.".into());
            }
            list.filter = selected.into();
            match list.kind {
                Kind::Assigned | Kind::Created | Kind::Subscribed => {
                    list.kind = match selected {
                        "assigned" => Kind::Assigned,
                        "created" => Kind::Created,
                        _ => Kind::Subscribed,
                    };
                }
                Kind::Cycle => {
                    list.target = session
                        .cycle_teams
                        .iter()
                        .find(|t| t["id"] == selected)
                        .map(|team| string(&team["activeCycle"], "id"))
                        .unwrap_or_default();
                }
                Kind::Notifications => return Ok(()),
                _ => {}
            }
            list.load(api, prefs, false).await?;
        }
        Action::Detail(entity, value) => {
            session.push(Screen::Detail(entity, item(api, entity, &value).await?))?
        }
        Action::List(kind, target) => {
            let mut list = Listing::new(kind, target);
            list.load(api, prefs, false).await?;
            session.push(Screen::List(list))?;
        }
        Action::Form(kind, value, mut context) => {
            let value = match kind {
                FormKind::Issue | FormKind::Project if value["id"].is_string() => {
                    item(
                        api,
                        if kind == FormKind::Issue {
                            Entity::Issue
                        } else {
                            Entity::Project
                        },
                        &value,
                    )
                    .await?
                }
                _ => value,
            };
            if kind == FormKind::Issue && context["assignSelf"] == true {
                context["assigneeId"] = model::viewer(api).await?["id"].clone();
            }
            let mut form = Form::new(kind, value, context);
            form.load(api, prefs).await?;
            session.push(Screen::Form(form))?;
        }
        Action::FormChange(field) => {
            let Screen::Form(form) = &mut session.screen else {
                return Err("No Linear form is open.".into());
            };
            form.apply(&params["formValues"]);
            if let Some(value) = params["formValues"]["value"].as_str() {
                form.values[&field] = json!(value);
            }
            if field == "teamId" {
                for name in [
                    "stateId",
                    "cycleId",
                    "projectId",
                    "projectMilestoneId",
                    "labelIds",
                ] {
                    form.values[name] = if name == "labelIds" {
                        json!([])
                    } else {
                        json!("")
                    };
                }
                form.load_team(api, prefs).await?;
            } else if field == "projectId" {
                form.values["projectMilestoneId"] = json!("");
                form.load_milestones(api).await?;
            }
        }
        Action::Submit => {
            let Screen::Form(form) = &mut session.screen else {
                return Err("No Linear form is open.".into());
            };
            form.apply(&params["formValues"]);
            let saved = form.submit(api).await?;
            session.screen = Screen::Result(saved.kind, saved.entity, saved.message.into());
        }
        Action::AssignSelf(id) => {
            let viewer = model::viewer(api).await?;
            api.mutation(
                Mutation::IssueUpdate,
                Some(&id),
                Some(json!({"assigneeId":viewer["id"]})),
            )
            .await?;
            session.notice = "Assigned to you.".into();
            if let Screen::Detail(Entity::Issue, current) = &mut session.screen {
                current["assignee"] = viewer;
            }
            refresh(api, prefs, session).await;
        }
        Action::Subscribe(id, subscribe) => {
            let field = if subscribe {
                "issueSubscribe"
            } else {
                "issueUnsubscribe"
            };
            let query = format!(
                "mutation SubscribeIssue($id: String!) {{ {field}(id: $id) {{ success }} }}"
            );
            let data = api.request(&query, json!({"id":id})).await?;
            if data[field]["success"] != true {
                return Err("Linear did not confirm the subscription change.".into());
            }
            session.notice = if subscribe {
                "Subscribed to issue."
            } else {
                "Unsubscribed from issue."
            }
            .into();
        }
        Action::Archive(entity, id) => {
            let title = if entity == Entity::Issue {
                "Archive Issue"
            } else {
                "Archive Project"
            };
            let answer = ctx
                .host(
                    "host.dialog.confirm",
                    json!({"title":title,"message":"Archive this item in Linear?",
                "buttons":["Cancel","Archive"],"type":"warning"}),
                )
                .await?;
            if answer["response"] != 1 {
                return Ok(());
            }
            let mutation = if entity == Entity::Issue {
                Mutation::IssueArchive
            } else {
                Mutation::ProjectArchive
            };
            api.mutation(mutation, Some(&id), None).await?;
            if let Some(screen) = session.history.pop() {
                session.screen = screen;
            }
            session.notice = "Archived in Linear.".into();
            refresh(api, prefs, session).await;
        }
        Action::Notification(id, action) => {
            let Screen::List(list) = &mut session.screen else {
                return Err("Linear notification list is closed.".into());
            };
            let index = list
                .rows
                .iter()
                .position(|row| row["id"] == id)
                .ok_or("This notification has changed.")?;
            let notifications = &mut list.rows;
            if matches!(action, NoteAction::Open) {
                forms::open(
                    ctx,
                    &tray::notification_url(&notifications[index], &list.organization),
                )
                .await?;
            }
            if matches!(action, NoteAction::Archive) {
                api.mutation(Mutation::NotificationArchive, Some(&id), None)
                    .await?;
                notifications.remove(index);
            } else {
                let (field, stamp) = match action {
                    NoteAction::Unread => ("readAt", Value::Null),
                    NoteAction::Snooze => ("snoozedUntilAt", forms::now(1)),
                    _ => ("readAt", forms::now(0)),
                };
                let mut input = json!({});
                input[field] = stamp.clone();
                api.mutation(Mutation::NotificationUpdate, Some(&id), Some(input))
                    .await?;
                notifications[index][field] = stamp;
            }
        }
        Action::OpenInbox => {
            let Screen::List(list) = &session.screen else {
                return Ok(());
            };
            forms::open(ctx, &workspace_url(&list.organization, "inbox")).await?;
        }
    }
    Ok(())
}

impl Extension for Linear {
    fn id(&self) -> &'static str {
        "com.tezbar.native.linear"
    }
    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
    fn commands(&self) -> &'static [&'static str] {
        COMMANDS
    }
    fn concurrency(&self, _: &str) -> CommandConcurrency {
        CommandConcurrency::RejectWhileBusy
    }

    fn initialize<'a>(&'a self, ctx: &'a ExtensionContext) -> BoxFuture<'a, CoreResult> {
        Box::pin(async move {
            self.background
                .set(ctx.clone())
                .map_err(|_| "Linear was already initialized.")?;
            Ok(json!({}))
        })
    }

    fn background_event<'a>(
        &'a self,
        ctx: &'a ExtensionContext,
        payload: &'a Value,
    ) -> BoxFuture<'a, CoreResult> {
        Box::pin(async move { self.tray.event(ctx, payload).await })
    }

    fn handle<'a>(
        &'a self,
        ctx: &'a ExtensionContext,
        command: &'a str,
        op: &'a str,
        payload: &'a Value,
        state: &'a mut Value,
    ) -> BoxFuture<'a, CoreResult> {
        Box::pin(async move {
            if op == "extension:run-command" {
                return self.run(ctx, command, payload, state).await;
            }
            if op == "extension:dispose-session" {
                return Ok(unchanged());
            }
            let prefs = &payload["preferences"];
            if state.get("screen").is_none() {
                if payload["actionId"] == "linear-settings" && op == "extension:invoke-action" {
                    forms::settings(ctx).await?;
                }
                return Ok(unchanged());
            }
            let api = self
                .transport
                .authenticate(prefs["linearApiKey"].as_str().unwrap_or_default().trim())?;
            let mut session: Session =
                serde_json::from_value(state.clone()).map_err(|_| "Linear session is invalid.")?;
            session.notice.clear();
            match op {
                "extension:invoke-action" => {
                    let id = payload["actionId"].as_str().unwrap_or_default();
                    let Some(action) = session.actions.remove(id) else {
                        return Ok(unchanged());
                    };
                    if let Err(error) =
                        perform(&api, ctx, prefs, &mut session, action, payload).await
                    {
                        session.notice = error;
                    }
                }
                "extension:search-text-changed" => {
                    let Screen::List(list) = &mut session.screen else {
                        return Ok(unchanged());
                    };
                    if !list.kind.remote_search() {
                        return Ok(unchanged());
                    }
                    let mut updated = list.clone();
                    updated.query = search_text(payload);
                    updated.load(&api, prefs, false).await?;
                    *list = updated;
                }
                "extension:load-more" | "extension:refresh-session" => {
                    let Screen::List(list) = &mut session.screen else {
                        return Ok(unchanged());
                    };
                    let more = op == "extension:load-more";
                    if more && list.cursor.is_none() {
                        return Ok(unchanged());
                    }
                    list.load(&api, prefs, more).await?;
                }
                _ => return Ok(unchanged()),
            }
            finish(&mut session, prefs, state)
        })
    }
}

#[tokio::main(worker_threads = 2)]
async fn main() {
    let result = match Transport::new() {
        Ok(transport) => {
            let tray = tray::Tray::new(transport.clone());
            serve(Linear {
                transport,
                tray,
                background: OnceLock::new(),
            })
            .await
        }
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
