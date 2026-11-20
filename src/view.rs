use crate::{
    api::string,
    model::{
        workspace_url, Action, Choice, Entity, Form, FormKind, Kind, Listing, NoteAction, Screen,
        Session,
    },
};
use serde_json::{json, Value};
use std::collections::HashMap;
use tezbar_extension_sdk::view;

pub const ICON: &str = "resources/linear-app-icon.png";

struct Actions {
    revision: u64,
    operations: HashMap<String, Action>,
    nodes: Vec<Value>,
}

impl Actions {
    fn add(&mut self, title: &str, operation: Action) -> String {
        self.register(title, "action", operation)
    }

    fn register(&mut self, title: &str, kind: &str, operation: Action) -> String {
        let id = format!("linear-{}-{}", self.revision, self.nodes.len());
        self.nodes.push(json!({"id":id,"title":title,"kind":kind}));
        self.operations.insert(id.clone(), operation);
        id
    }
}

pub fn render(session: &mut Session, prefs: &Value) -> Value {
    session.revision = session.revision.wrapping_add(1);
    let mut actions = Actions {
        revision: session.revision,
        operations: HashMap::new(),
        nodes: Vec::new(),
    };
    let back = (!session.history.is_empty()).then(|| actions.add("Go Back", Action::Back));
    let (title, mut root) = match &session.screen {
        Screen::List(list) => (list.kind.title().into(), listing(list, prefs, &mut actions)),
        Screen::Detail(kind, entity) => detail(*kind, entity, prefs, &mut actions),
        Screen::Form(form) => (
            form.kind.title(form.editing()).into(),
            form_node(form, prefs, &mut actions),
        ),
        Screen::Result(kind, entity, message) => {
            result(*kind, entity, message, prefs, &mut actions)
        }
    };
    if let Some(back) = back {
        root["props"]["backActionId"] = json!(back);
    }
    if !session.notice.is_empty() {
        if root["type"] == "Form" {
            root["children"]
                .as_array_mut()
                .expect("form children")
                .insert(
                    0,
                    json!({"type":"Form.Description","props":{"text":session.notice}}),
                );
        }
        root["props"]["subtitle"] = json!(session.notice);
    }
    session.actions = actions.operations;
    let mut reply = view(&title, root, actions.nodes);
    if !session.notice.is_empty() {
        reply["notice"] = json!(session.notice);
    }
    reply
}

fn listing(list: &Listing, prefs: &Value, actions: &mut Actions) -> Value {
    let mut global = vec![];
    let create = if list.kind.issues() {
        let context = match list.kind {
            Kind::Cycle => json!({"teamId":list.filter,"cycleId":list.target}),
            Kind::ProjectIssues => json!({"projectId":list.target}),
            Kind::SubIssues => json!({"parentId":list.target}),
            Kind::MilestoneIssues => json!({"projectMilestoneId":list.target}),
            Kind::Assigned => json!({"assignSelf":true}),
            _ => json!({}),
        };
        Some((
            "Create Issue",
            Action::Form(FormKind::Issue, Value::Null, context),
        ))
    } else {
        match list.kind {
            Kind::Projects => Some((
                "Create Project",
                Action::Form(
                    FormKind::Project,
                    Value::Null,
                    json!({"teamId":list.filter}),
                ),
            )),
            Kind::Comments => Some((
                "Add Comment",
                Action::Form(
                    FormKind::Comment,
                    Value::Null,
                    json!({"issueId":list.target}),
                ),
            )),
            Kind::Attachments => Some((
                "Add Link Attachment",
                Action::Form(
                    FormKind::Attachment,
                    Value::Null,
                    json!({"issueId":list.target}),
                ),
            )),
            Kind::Milestones => Some((
                "Create Milestone",
                Action::Form(
                    FormKind::Milestone,
                    Value::Null,
                    json!({"projectId":list.target}),
                ),
            )),
            Kind::ProjectDocuments => Some((
                "Create Document",
                Action::Form(
                    FormKind::Document,
                    Value::Null,
                    json!({"projectId":list.target}),
                ),
            )),
            _ => None,
        }
    };
    if let Some((title, action)) = create {
        global.push(actions.add(title, action));
    }
    if matches!(list.kind, Kind::Notifications | Kind::Unread) {
        global.push(actions.add("Open Linear Inbox", Action::OpenInbox));
    }
    if list.cursor.is_some() {
        global.push(actions.add("Load More", Action::More));
    }
    global.push(actions.add("Refresh", Action::Refresh));
    if list.kind == Kind::Notifications {
        global.push(actions.add("Configure Linear", Action::Settings));
    }
    let mut props = json!({
        "searchBarPlaceholder": if list.kind.remote_search() { "Search Linear" } else { "Filter results" },
        "__hasServerSearch": list.kind.remote_search(),
        "pagination":{"hasMore":list.cursor.is_some()},
        "actionIds":global,
    });
    if !list.options.is_empty() {
        let children: Vec<_> = list
            .options
            .iter()
            .map(|item| {
                json!({
                    "type":"List.Dropdown.Item","props":{"title":item.title,"value":item.id},
                })
            })
            .collect();
        props["searchBarAccessory"] = json!({
            "type":"List.Dropdown",
            "props":{"actionId":actions.add("Change Filter",Action::Select),"value":list.filter},
            "children":children,
        });
    }

    let mut children: Vec<Value> = list
        .rows
        .iter()
        .filter_map(|entity| row(list, entity, prefs, actions))
        .collect();
    if children.is_empty() && list.cursor.is_some() {
        children.push(
            json!({"type":"List.Item","props":{"id":"more-linear-results","title":"Load More",
            "actionIds":[actions.add("Load More",Action::More)]}}),
        );
    } else if children.is_empty() {
        children.push(json!({"type":"List.EmptyView","props":{
            "title":if list.kind == Kind::Cycle && list.target.is_empty() { "No Active Cycle" } else { "No Results" },
            "icon":ICON,
        }}));
    }
    json!({"type":"List","props":props,"children":children})
}

fn row(list: &Listing, item: &Value, prefs: &Value, actions: &mut Actions) -> Option<Value> {
    let kind = list.kind;
    let id = item["id"].as_str()?;
    let mut url = string(item, "url");
    let (entity, data, title, subtitle) = if kind.issues() {
        let subtitle = join(&[
            string(item, "identifier"),
            string(&item["state"], "name"),
            string(&item["assignee"], "displayName"),
            string(item, "priorityLabel"),
        ]);
        (Entity::Issue, item.clone(), string(item, "title"), subtitle)
    } else {
        match kind {
            Kind::Projects => (
                Entity::Project,
                item.clone(),
                string(item, "name"),
                join(&[
                    string(&item["status"], "name"),
                    string(&item["lead"], "displayName"),
                ]),
            ),
            Kind::Views => (
                Entity::View,
                item.clone(),
                string(item, "name"),
                string(&item["team"], "name"),
            ),
            Kind::Documents | Kind::ProjectDocuments => (
                Entity::Document,
                item.clone(),
                string(item, "title"),
                join(&[
                    string(&item["project"], "name"),
                    string(&item["initiative"], "name"),
                    string(&item["creator"], "displayName"),
                ]),
            ),
            Kind::Favorites => {
                let (entity, mut data, title) = favorite(item)?;
                if url.is_empty() {
                    url = string(&data, "url");
                }
                if url.is_empty() {
                    let path = match item["type"].as_str() {
                        Some("customView") => format!("view/{}", string(&data, "id")),
                        Some("document") => format!("document/{}", string(&data, "id")),
                        Some("initiative") => format!("initiative/{}", string(&data, "id")),
                        Some("cycle") => format!(
                            "team/{}/cycle/{}",
                            string(&data["team"], "key"),
                            data["number"]
                        ),
                        _ => String::new(),
                    };
                    if !path.is_empty() {
                        url = workspace_url(&list.organization, &path);
                    }
                }
                data["url"] = json!(url);
                (entity, data, title, string(item, "type"))
            }
            Kind::Notifications | Kind::Unread => {
                let unread = unread(item);
                let snoozed = snoozed(item);
                if (kind == Kind::Unread && !unread)
                    || (list.filter == "unread" && !unread)
                    || (list.filter == "snoozed" && !snoozed)
                {
                    return None;
                }
                let name = join(&[
                    string(&item["actor"], "displayName"),
                    string(&item["botActor"], "name"),
                ]);
                let state = if snoozed {
                    "Snoozed"
                } else if unread {
                    "Unread"
                } else {
                    "Read"
                };
                let title = string(item, "title");
                let subtitle = join(&[string(item, "subtitle"), name, state.into()]);
                let mut ids = vec![
                    actions.add(
                        "Open Notification",
                        Action::Notification(id.into(), NoteAction::Open),
                    ),
                    actions.add(
                        "View Details",
                        Action::Detail(Entity::Notification, item.clone()),
                    ),
                ];
                let (label, action) = if unread {
                    ("Mark as Read", NoteAction::Read)
                } else {
                    ("Mark as Unread", NoteAction::Unread)
                };
                ids.push(actions.add(label, Action::Notification(id.into(), action)));
                ids.push(actions.add(
                    "Snooze for One Hour",
                    Action::Notification(id.into(), NoteAction::Snooze),
                ));
                ids.push(actions.add(
                    "Archive Notification",
                    Action::Notification(id.into(), NoteAction::Archive),
                ));
                if let Some(issue) = item.get("issue").filter(|value| value["id"].is_string()) {
                    ids.push(actions.add(
                        "Show Issue Details",
                        Action::Detail(Entity::Issue, issue.clone()),
                    ));
                }
                return Some(list_row(id, &title, &subtitle, &ids));
            }
            Kind::Milestones => {
                let ids = vec![actions.add(
                    "Show Milestone Issues",
                    Action::List(Kind::MilestoneIssues, id.into()),
                )];
                return Some(list_row(
                    id,
                    &string(item, "name"),
                    &string(item, "targetDate"),
                    &ids,
                ));
            }
            Kind::Comments => (
                Entity::Text,
                item.clone(),
                summary(&string(item, "body")),
                string(&item["user"], "displayName"),
            ),
            Kind::ProjectUpdates => (
                Entity::Text,
                item.clone(),
                summary(&string(item, "body")),
                join(&[string(item, "health"), string(item, "createdAt")]),
            ),
            Kind::Attachments => (
                Entity::Text,
                item.clone(),
                string(item, "title"),
                string(item, "subtitle"),
            ),
            _ => return None,
        }
    };
    let mut ids = Vec::new();
    let add_detail =
        !matches!(kind, Kind::Attachments) && (entity != Entity::Text || kind != Kind::Favorites);
    if !url.is_empty() && (prefs["primaryAction"] == "openInLinear" || !add_detail) {
        ids.push(actions.add("Open in Linear", Action::Open(url.clone())));
    }
    if add_detail {
        ids.push(actions.add("Show Details", Action::Detail(entity, data.clone())));
    }
    if !url.is_empty() && prefs["primaryAction"] != "openInLinear" && add_detail {
        ids.push(actions.add("Open in Linear", Action::Open(url.clone())));
    }
    if entity == Entity::Issue {
        if kind != Kind::Favorites {
            ids.push(actions.add(
                "Edit Issue",
                Action::Form(FormKind::Issue, data.clone(), json!({})),
            ));
        }
        add_issue_copy(&data, prefs, actions, &mut ids);
    } else if !url.is_empty() {
        ids.push(actions.add("Copy Link", Action::Copy(url)));
    }
    Some(list_row(id, &title, &subtitle, &ids))
}

fn favorite(item: &Value) -> Option<(Entity, Value, String)> {
    let field = item["type"].as_str()?;
    let (entity, key) = match field {
        "issue" => (Entity::Issue, "title"),
        "project" => (Entity::Project, "name"),
        "document" => (Entity::Document, "title"),
        "customView" => (Entity::View, "name"),
        "user" => (Entity::Text, "displayName"),
        "cycle" | "initiative" | "label" => (Entity::Text, "name"),
        _ => return None,
    };
    let data = item[field].clone();
    if data.is_null() {
        return None;
    }
    let mut title = string(&data, key);
    if title.is_empty() && field == "cycle" {
        title = format!("{} Cycle {}", string(&data["team"], "key"), data["number"]);
    }
    Some((entity, data, title))
}

fn list_row(id: &str, title: &str, subtitle: &str, ids: &[String]) -> Value {
    json!({"type":"List.Item","props":{"id":id,"title":title,"subtitle":subtitle,"icon":ICON,"actionIds":ids}})
}

fn add_issue_copy(item: &Value, prefs: &Value, actions: &mut Actions, ids: &mut Vec<String>) {
    for (field, label) in [
        ("identifier", "Copy Issue ID"),
        ("title", "Copy Issue Title"),
        ("url", "Copy Issue URL"),
        ("branchName", "Copy Git Branch Name"),
    ] {
        if let Some(value) = item[field].as_str().filter(|value| !value.is_empty()) {
            ids.push(actions.add(label, Action::Copy(value.into())));
        }
    }
    if let Some(format) = prefs["issueCustomCopyAction"]
        .as_str()
        .filter(|text| !text.is_empty())
    {
        ids.push(actions.add(
            "Copy Custom Issue Format",
            Action::Copy(issue_format(format, item)),
        ));
    }
}

pub fn issue_format(format: &str, value: &Value) -> String {
    let mut output = String::new();
    let mut remaining = format;
    while let Some(start) = remaining.find('{') {
        output.push_str(&remaining[..start]);
        remaining = &remaining[start..];
        let Some(end) = remaining.find('}') else {
            break;
        };
        let marker = &remaining[..=end];
        let field = match marker {
            "{ISSUE_TITLE}" => "title",
            "{ISSUE_ID}" => "identifier",
            "{ISSUE_URL}" => "url",
            "{ISSUE_BRANCH_NAME}" => "branchName",
            _ => "",
        };
        if field.is_empty() {
            output.push_str(marker);
        } else {
            output.push_str(value[field].as_str().unwrap_or_default());
        }
        remaining = &remaining[end + 1..];
    }
    output.push_str(remaining);
    output
}

fn detail(kind: Entity, item: &Value, prefs: &Value, actions: &mut Actions) -> (String, Value) {
    let id = string(item, "id");
    let url = string(item, "url");
    let mut ids = Vec::new();
    if !url.is_empty() {
        ids.push(actions.add("Open in Linear", Action::Open(url.clone())));
    }
    let (title, body, facts) = match kind {
        Entity::Issue => {
            ids.push(actions.add(
                "Edit Issue",
                Action::Form(FormKind::Issue, item.clone(), json!({})),
            ));
            ids.push(actions.add("Assign to Me", Action::AssignSelf(id.clone())));
            ids.push(actions.add("Subscribe", Action::Subscribe(id.clone(), true)));
            ids.push(actions.add("Unsubscribe", Action::Subscribe(id.clone(), false)));
            for (title, kind) in [
                ("Show Comments", Kind::Comments),
                ("Show Attachments", Kind::Attachments),
                ("Show Sub-issues", Kind::SubIssues),
            ] {
                ids.push(actions.add(title, Action::List(kind, id.clone())));
            }
            ids.push(actions.add(
                "Add Comment",
                Action::Form(FormKind::Comment, Value::Null, json!({"issueId":id})),
            ));
            ids.push(actions.add(
                "Add Link Attachment",
                Action::Form(FormKind::Attachment, Value::Null, json!({"issueId":id})),
            ));
            ids.push(actions.add(
                "Create Sub-issue",
                Action::Form(
                    FormKind::Issue,
                    Value::Null,
                    json!({"parentId":id,"teamId":item["team"]["id"]}),
                ),
            ));
            ids.push(actions.add(
                "Set Parent Issue",
                Action::Form(FormKind::Parent, item.clone(), json!({})),
            ));
            if item["archivedAt"].is_null() {
                ids.push(actions.add("Archive Issue", Action::Archive(Entity::Issue, id.clone())));
            }
            add_issue_copy(item, prefs, actions, &mut ids);
            let title = format!("{} {}", string(item, "identifier"), string(item, "title"))
                .trim()
                .to_owned();
            (
                title,
                string(item, "description"),
                vec![
                    ("Status", string(&item["state"], "name")),
                    ("Assignee", string(&item["assignee"], "displayName")),
                    ("Team", string(&item["team"], "name")),
                    ("Priority", string(item, "priorityLabel")),
                    ("Cycle", string(&item["cycle"], "name")),
                    ("Parent", string(&item["parent"], "identifier")),
                    ("Project", string(&item["project"], "name")),
                    ("Milestone", string(&item["projectMilestone"], "name")),
                    ("Due", string(item, "dueDate")),
                ],
            )
        }
        Entity::Project => {
            for (label, kind) in [
                ("Show Project Issues", Kind::ProjectIssues),
                ("Show Project Updates", Kind::ProjectUpdates),
                ("Show Milestones", Kind::Milestones),
                ("Show Documents", Kind::ProjectDocuments),
            ] {
                ids.push(actions.add(label, Action::List(kind, id.clone())));
            }
            ids.push(actions.add(
                "Edit Project",
                Action::Form(FormKind::Project, item.clone(), json!({})),
            ));
            ids.push(actions.add(
                "Archive Project",
                Action::Archive(Entity::Project, id.clone()),
            ));
            let progress = item["progress"]
                .as_f64()
                .map(|value| format!("{:.0}%", value * 100.))
                .unwrap_or_default();
            (
                string(item, "name"),
                string(item, "description"),
                vec![
                    ("Status", string(&item["status"], "name")),
                    ("Lead", string(&item["lead"], "displayName")),
                    ("Progress", progress),
                    ("Start", string(item, "startDate")),
                    ("Target", string(item, "targetDate")),
                ],
            )
        }
        Entity::View => {
            ids.insert(
                0,
                actions.add("Show View Issues", Action::List(Kind::ViewIssues, id)),
            );
            (
                string(item, "name"),
                String::new(),
                vec![("Team", string(&item["team"], "name"))],
            )
        }
        Entity::Document => {
            ids.push(actions.add(
                "Edit Document",
                Action::Form(FormKind::Document, item.clone(), json!({})),
            ));
            (
                string(item, "title"),
                string(item, "content"),
                vec![
                    ("Project", string(&item["project"], "name")),
                    ("Initiative", string(&item["initiative"], "name")),
                    ("Creator", string(&item["creator"], "displayName")),
                    ("Updated", string(item, "updatedAt")),
                ],
            )
        }
        Entity::Notification => (
            string(item, "title"),
            join(&[string(item, "subtitle"), string(&item["comment"], "body")]),
            vec![
                ("Created", string(item, "createdAt")),
                ("Read", string(item, "readAt")),
            ],
        ),
        Entity::Text => (
            string(item, "title"),
            string(item, "body"),
            vec![("Author", string(&item["user"], "displayName"))],
        ),
    };
    if !url.is_empty() && kind != Entity::Issue {
        ids.push(actions.add("Copy Link", Action::Copy(url)));
    }
    let mut markdown = format!("# {}\n\n{}", escape(&title), body);
    for (label, value) in facts {
        if !value.is_empty() {
            markdown.push_str(&format!("\n\n**{label}** {}", escape(&value)));
        }
    }
    (
        title,
        json!({"type":"Detail","props":{"markdown":markdown,"actionIds":ids}}),
    )
}

fn result(
    kind: Entity,
    item: &Value,
    message: &str,
    prefs: &Value,
    actions: &mut Actions,
) -> (String, Value) {
    let mut ids = vec![];
    let url = string(item, "url");
    if !url.is_empty() {
        ids.push(actions.add("Open in Linear", Action::Open(url.clone())));
    }
    if kind == Entity::Issue {
        let id = string(item, "identifier");
        let title = string(item, "title");
        let (label, value) = match prefs["copyToastAction"].as_str().unwrap_or("id") {
            "id-as-link" => ("Copy Issue ID as Link", format!("[{}]({url})", escape(&id))),
            "title-as-link" => (
                "Copy Issue Title as Link",
                format!("[{}]({url})", escape(&title)),
            ),
            "url" => ("Copy Issue URL", url),
            "title" => ("Copy Issue Title", title),
            _ => ("Copy Issue ID", id),
        };
        ids.push(actions.add(label, Action::Copy(value)));
    }
    ids.push(actions.add("Continue", Action::Back));
    let title = join(&[
        string(item, "identifier"),
        string(item, "title"),
        string(item, "name"),
    ]);
    (
        message.into(),
        json!({"type":"Detail","props":{
            "markdown":format!("# {}\n\n{}",escape(message),escape(&title)),"actionIds":ids,
        }}),
    )
}

fn form_node(form: &Form, prefs: &Value, actions: &mut Actions) -> Value {
    let editing = form.editing();
    let kind = form.kind;
    let mut nodes = vec![];
    let mut field = |node: &str,
                     id: &str,
                     title: &str,
                     required: bool,
                     value: Value,
                     options: Vec<Value>,
                     callback: Option<String>| {
        let mut props = json!({"id":id,"title":title,"required":required});
        if !value.is_null() {
            props["value"] = value;
        }
        if let Some(callback) = callback {
            props["actionId"] = json!(callback);
        }
        if prefs["autofocusField"] == id || (prefs["autofocusField"].is_null() && id == "title") {
            props["autoFocus"] = json!(true);
        }
        nodes.push(json!({"type":node,"props":props,"children":options}));
    };
    match kind {
        FormKind::Issue => {
            let change = actions.add("Change Team", Action::FormChange("teamId".into()));
            let project = actions.add("Change Project", Action::FormChange("projectId".into()));
            field(
                "Form.Dropdown",
                "teamId",
                "Team",
                true,
                form.values["teamId"].clone(),
                options(form, "teamId", None, false),
                Some(change),
            );
            field(
                "Form.TextField",
                "title",
                "Title",
                true,
                form.values["title"].clone(),
                vec![],
                None,
            );
            field(
                "Form.TextArea",
                "description",
                "Description",
                false,
                form.values["description"].clone(),
                vec![],
                None,
            );
            for (id, title, empty, callback) in [
                ("stateId", "Status", "Default", None),
                ("assigneeId", "Assignee", "Unassigned", None),
                ("cycleId", "Cycle", "No Cycle", None),
                ("projectId", "Project", "No Project", Some(project)),
                ("projectMilestoneId", "Milestone", "No Milestone", None),
            ] {
                field(
                    "Form.Dropdown",
                    id,
                    title,
                    false,
                    form.values[id].clone(),
                    options(form, id, Some(empty), false),
                    callback,
                );
            }
            let priorities: Vec<_> = ["No Priority", "Urgent", "High", "Medium", "Low"]
                .into_iter()
                .enumerate()
                .map(|(index, title)| choice(&Choice::new(index.to_string(), title), false))
                .collect();
            let priority = form.values["priority"]
                .as_i64()
                .or_else(|| form.values["priority"].as_str()?.parse().ok())
                .unwrap_or_default()
                .to_string();
            field(
                "Form.Dropdown",
                "priority",
                "Priority",
                false,
                json!(priority),
                priorities,
                None,
            );
            field(
                "Form.TagPicker",
                "labelIds",
                "Labels",
                false,
                form.values["labelIds"].clone(),
                options(form, "labelIds", None, true),
                None,
            );
            let estimate = form.values["estimate"]
                .as_i64()
                .map(|number| json!(number.to_string()))
                .unwrap_or(form.values["estimate"].clone());
            field(
                "Form.TextField",
                "estimate",
                "Estimate",
                false,
                estimate,
                vec![],
                None,
            );
            field(
                "Form.DatePicker",
                "dueDate",
                "Due Date",
                false,
                form.values["dueDate"].clone(),
                vec![],
                None,
            );
        }
        FormKind::Project => {
            for (id, title, required) in
                [("teamIds", "Teams", true), ("memberIds", "Members", false)]
            {
                field(
                    "Form.TagPicker",
                    id,
                    title,
                    required,
                    form.values[id].clone(),
                    options(form, id, None, true),
                    None,
                );
            }
            field(
                "Form.TextField",
                "name",
                "Name",
                true,
                form.values["name"].clone(),
                vec![],
                None,
            );
            field(
                "Form.TextArea",
                "description",
                "Description",
                false,
                form.values["description"].clone(),
                vec![],
                None,
            );
            for (id, title, empty) in [
                ("statusId", "Status", "Default"),
                ("leadId", "Lead", "Unassigned"),
            ] {
                field(
                    "Form.Dropdown",
                    id,
                    title,
                    false,
                    form.values[id].clone(),
                    options(form, id, Some(empty), false),
                    None,
                );
            }
            field(
                "Form.DatePicker",
                "startDate",
                "Start Date",
                false,
                form.values["startDate"].clone(),
                vec![],
                None,
            );
            field(
                "Form.DatePicker",
                "targetDate",
                "Target Date",
                false,
                form.values["targetDate"].clone(),
                vec![],
                None,
            );
        }
        FormKind::Comment => field(
            "Form.TextArea",
            "body",
            "Comment",
            true,
            form.values["body"].clone(),
            vec![],
            None,
        ),
        FormKind::Milestone => {
            field(
                "Form.TextField",
                "name",
                "Name",
                true,
                form.values["name"].clone(),
                vec![],
                None,
            );
            field(
                "Form.TextArea",
                "description",
                "Description",
                false,
                form.values["description"].clone(),
                vec![],
                None,
            );
            field(
                "Form.DatePicker",
                "targetDate",
                "Target Date",
                false,
                form.values["targetDate"].clone(),
                vec![],
                None,
            );
        }
        FormKind::Attachment => {
            field(
                "Form.TextField",
                "url",
                "URL",
                true,
                form.values["url"].clone(),
                vec![],
                None,
            );
            field(
                "Form.TextField",
                "title",
                "Title",
                true,
                form.values["title"].clone(),
                vec![],
                None,
            );
            field(
                "Form.TextField",
                "subtitle",
                "Subtitle",
                false,
                form.values["subtitle"].clone(),
                vec![],
                None,
            );
        }
        FormKind::Document => {
            field(
                "Form.TextField",
                "title",
                "Title",
                true,
                form.values["title"].clone(),
                vec![],
                None,
            );
            field(
                "Form.TextArea",
                "content",
                "Content",
                false,
                form.values["content"].clone(),
                vec![],
                None,
            );
        }
        FormKind::Parent => field(
            "Form.TextField",
            "identifier",
            "Parent Issue ID",
            false,
            form.values["identifier"].clone(),
            vec![],
            None,
        ),
    }
    let submit = actions.register(kind.title(editing), "submit-form", Action::Submit);
    json!({"type":"Form","props":{"actionIds":[submit]},"children":nodes})
}

fn options(form: &Form, id: &str, empty: Option<&str>, multiple: bool) -> Vec<Value> {
    let mut options = Vec::new();
    if let Some(empty) = empty {
        options.push(choice(&Choice::new("", empty), multiple));
    }
    if let Some(rows) = form.options.get(id) {
        options.extend(rows.iter().map(|item| choice(item, multiple)));
    }
    options
}

fn choice(item: &Choice, multiple: bool) -> Value {
    json!({"type":if multiple {"Form.TagPicker.Item"} else {"Form.Dropdown.Item"},
        "props":{"title":item.title,"value":item.id}})
}

pub fn auth_required(message: &str) -> Value {
    view(
        "Linear",
        json!({"type":"Detail","props":{"markdown":format!("# Connect Linear\n\n{message}"),
        "actionIds":["linear-settings"]}}),
        vec![json!({"id":"linear-settings","title":"Open Extension Settings","kind":"action"})],
    )
}

pub fn join(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|value| !value.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" · ")
}

fn escape(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if matches!(
                c,
                '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '<' | '>' | '#'
            ) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}

fn summary(text: &str) -> String {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .chars()
        .take(130)
        .collect()
}

pub fn unread(item: &Value) -> bool {
    item["readAt"].is_null() && !snoozed(item)
}

fn snoozed(item: &Value) -> bool {
    let Some(text) = item["snoozedUntilAt"].as_str() else {
        return false;
    };
    time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339)
        .is_ok_and(|stamp| stamp > time::OffsetDateTime::now_utc())
}
