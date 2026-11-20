use crate::api::{self, string, Api, Connection, Page};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    SearchIssues,
    Assigned,
    Created,
    Subscribed,
    Cycle,
    Projects,
    Views,
    Documents,
    Notifications,
    Unread,
    Favorites,
    ViewIssues,
    ProjectIssues,
    SubIssues,
    MilestoneIssues,
    Comments,
    Attachments,
    ProjectUpdates,
    Milestones,
    ProjectDocuments,
}

impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Self::SearchIssues => "Search Issues",
            Self::Assigned | Self::Subscribed => "My Issues",
            Self::Created => "Created Issues",
            Self::Cycle => "Active Cycle",
            Self::Projects => "Search Projects",
            Self::Views => "Search Custom Views",
            Self::Documents => "Search Documents",
            Self::Notifications => "Notifications",
            Self::Unread => "Unread Notifications",
            Self::Favorites => "Favorites",
            Self::ViewIssues => "Custom View Issues",
            Self::ProjectIssues => "Project Issues",
            Self::SubIssues => "Sub-issues",
            Self::MilestoneIssues => "Milestone Issues",
            Self::Comments => "Comments",
            Self::Attachments => "Attachments",
            Self::ProjectUpdates => "Project Updates",
            Self::Milestones => "Milestones",
            Self::ProjectDocuments => "Project Documents",
        }
    }

    pub fn issues(self) -> bool {
        matches!(
            self,
            Self::SearchIssues
                | Self::Assigned
                | Self::Created
                | Self::Subscribed
                | Self::Cycle
                | Self::ViewIssues
                | Self::ProjectIssues
                | Self::SubIssues
                | Self::MilestoneIssues
        )
    }

    pub fn remote_search(self) -> bool {
        matches!(
            self,
            Self::SearchIssues | Self::Projects | Self::Documents | Self::ProjectDocuments
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Entity {
    Issue,
    Project,
    Document,
    View,
    Notification,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum FormKind {
    Issue,
    Project,
    Comment,
    Milestone,
    Attachment,
    Document,
    Parent,
}

impl FormKind {
    pub fn title(self, editing: bool) -> &'static str {
        match self {
            Self::Issue if editing => "Edit Issue",
            Self::Issue => "Create Issue",
            Self::Project if editing => "Edit Project",
            Self::Project => "Create Project",
            Self::Comment => "Add Comment",
            Self::Milestone => "Create Milestone",
            Self::Attachment => "Add Link Attachment",
            Self::Document if editing => "Edit Document",
            Self::Document => "Create Document",
            Self::Parent => "Set Parent Issue",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Choice {
    pub id: String,
    pub title: String,
}

impl Choice {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Listing {
    pub kind: Kind,
    pub target: String,
    pub filter: String,
    pub query: String,
    pub options: Vec<Choice>,
    pub rows: Vec<Value>,
    pub cursor: Option<String>,
    cursors: Vec<String>,
    pub organization: String,
}

impl Listing {
    pub fn new(kind: Kind, target: String) -> Self {
        Self {
            kind,
            target,
            filter: String::new(),
            query: String::new(),
            options: Vec::new(),
            rows: Vec::new(),
            cursor: None,
            cursors: Vec::new(),
            organization: String::new(),
        }
    }

    pub async fn load(
        &mut self,
        api: &Api<'_>,
        preferences: &Value,
        append: bool,
    ) -> Result<(), String> {
        if !append {
            self.rows.clear();
            self.cursor = None;
            self.cursors.clear();
        }
        if self.kind == Kind::Cycle && self.target.is_empty() {
            return Ok(());
        }
        let first = if self.kind.issues() {
            limit(preferences, "limit", 50, 50)
        } else {
            50
        };
        let (spec, args) = self.connection(preferences);
        let page = api.page(&spec, args, first, self.cursor.as_deref()).await?;
        self.accept(page, preferences)
    }

    fn accept(&mut self, page: Page, preferences: &Value) -> Result<(), String> {
        if let Some(next) = &page.next {
            if self.cursors.contains(next) {
                return Err("Linear returned an invalid pagination cursor.".into());
            }
            self.cursors.push(next.clone());
        }
        self.cursor = page.next;
        if !page.organization.is_empty() {
            self.organization = page.organization;
        }
        for mut value in page.rows {
            let id = string(&value, "id");
            if id.is_empty() || self.rows.iter().any(|item| item["id"] == id) {
                continue;
            }
            if self.kind == Kind::Views {
                if value["modelName"] != "Issue" {
                    continue;
                }
                value["url"] = json!(workspace_url(&self.organization, &format!("view/{id}")));
            }
            if self.kind.issues()
                && preferences["shouldHideRedundantIssues"] == true
                && redundant(&value)
            {
                continue;
            }
            self.rows.push(value);
        }
        if self.rows.len() >= 400 || self.cursors.len() >= 30 {
            self.cursor = None;
        }
        Ok(())
    }

    pub fn connection(&self, prefs: &Value) -> (Connection, Value) {
        use Kind::*;
        let target = &self.target;
        let term = &self.query;
        let redundant_filter = if prefs["shouldHideRedundantIssues"] == true {
            json!({"completedAt":{"null":true},"canceledAt":{"null":true}})
        } else {
            json!({})
        };
        match self.kind {
            SearchIssues if !term.is_empty() => (
                Connection::root("SearchIssues", "searchIssues", api::ISSUE)
                    .args(", $term: String!, $filter: IssueFilter", ", term: $term, filter: $filter, includeArchived: false"),
                json!({"term":term,"filter":redundant_filter}),
            ),
            SearchIssues | Subscribed | ProjectIssues | MilestoneIssues => {
                let mut filter = redundant_filter;
                match self.kind {
                    Subscribed => filter["subscribers"] = json!({"some":{"isMe":{"eq":true}}}),
                    ProjectIssues => filter["project"] = json!({"id":{"eq":target}}),
                    MilestoneIssues => filter["projectMilestone"] = json!({"id":{"eq":target}}),
                    _ => {}
                }
                (
                    Connection::root("Issues", "issues", api::ISSUE)
                        .args(", $filter: IssueFilter", ", filter: $filter, orderBy: updatedAt"),
                    json!({"filter":filter}),
                )
            }
            Assigned | Created => {
                let (name, field, path) = if self.kind == Assigned {
                    ("AssignedIssues", "assignedIssues", "viewer.assignedIssues")
                } else {
                    ("CreatedIssues", "createdIssues", "viewer.createdIssues")
                };
                (
                    Connection::root(name, field, api::ISSUE).nested("viewer",path)
                        .args(", $filter: IssueFilter", ", filter: $filter, orderBy: updatedAt"),
                    json!({"filter":redundant_filter}),
                )
            }
            Cycle | ViewIssues | SubIssues => {
                let (name, field, parent, path) = match self.kind {
                    Cycle => ("CycleIssues","issues","cycle(id: $id)","cycle.issues"),
                    ViewIssues => ("ViewIssues","issues","customView(id: $id)","customView.issues"),
                    _ => ("SubIssues","children","issue(id: $id)","issue.children"),
                };
                let spec = Connection::root(name, field, api::ISSUE).nested(parent,path);
                if self.kind == ViewIssues {
                    (spec.args(", $id: String!", ""), json!({"id":target}))
                } else {
                    (spec.args(", $id: String!, $filter: IssueFilter", ", filter: $filter"),
                        json!({"id":target, "filter":redundant_filter}))
                }
            }
            Projects => projects(term, &self.filter),
            Views => (
                Connection::root("CustomViews", "customViews", "id name modelName shared team { id name key }").organization(),
                json!({}),
            ),
            Documents | ProjectDocuments => {
                let mut filter = json!({"title":{"containsIgnoreCase":term}});
                if self.kind == ProjectDocuments {
                    filter["project"] = json!({"id":{"eq":target}});
                }
                (Connection::root("Documents","documents",api::DOCUMENT)
                    .args(", $filter: DocumentFilter", ", filter: $filter, orderBy: updatedAt"),
                    json!({"filter":filter}))
            }
            Unread if prefs["groupNotifications"] == true => (
                Connection::root("UnreadInbox","inboxNotifications",api::NOTIFICATION)
                    .args("",", unreadOnly: true").organization(), json!({})
            ),
            Notifications | Unread => (
                Connection::root("Notifications","notifications",
                    concat!("id type createdAt readAt snoozedUntilAt title subtitle url actor { displayName } botActor { name }",
                        " ... on IssueNotification { reactionEmoji comment { body url } issue { id identifier title url } }",
                        " ... on ProjectNotification { project { id name url } projectUpdate { url } }"))
                    .organization(), json!({})
            ),
            Favorites => (Connection::root("Favorites","favorites",api::FAVORITE).organization(),json!({})),
            Comments => (issue_connection("IssueComments","comments","issue.comments",api::COMMENT),json!({"id":target})),
            Attachments => (issue_connection("IssueAttachments","attachments","issue.attachments",api::ATTACHMENT),json!({"id":target})),
            ProjectUpdates => (project_connection("ProjectUpdates","projectUpdates","project.projectUpdates",api::UPDATE),json!({"id":target})),
            Milestones => (project_connection("Milestones","projectMilestones","project.projectMilestones",api::MILESTONE),json!({"id":target})),
        }
    }
}

fn issue_connection(
    name: &'static str,
    field: &'static str,
    path: &'static str,
    fields: &'static str,
) -> Connection {
    Connection::root(name, field, fields)
        .nested("issue(id: $id)", path)
        .args(", $id: String!", "")
}

fn project_connection(
    name: &'static str,
    field: &'static str,
    path: &'static str,
    fields: &'static str,
) -> Connection {
    Connection::root(name, field, fields)
        .nested("project(id: $id)", path)
        .args(", $id: String!", "")
}

fn redundant(value: &Value) -> bool {
    matches!(
        value["state"]["type"].as_str(),
        Some("completed" | "canceled" | "duplicate")
    )
}

pub fn workspace_url(org: &str, path: &str) -> String {
    if org.is_empty() || !org.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        String::new()
    } else {
        format!("https://linear.app/{org}/{path}")
    }
}

pub fn projects(term: &str, team: &str) -> (Connection, Value) {
    let spec = Connection::root("Projects", "projects", api::PROJECT);
    if team.is_empty() {
        (
            spec.args(
                ", $term: String!",
                ", filter: { name: { containsIgnoreCase: $term } }",
            ),
            json!({"term":term}),
        )
    } else {
        (
            spec.nested("team(id: $id)", "team.projects").args(
                ", $term: String!, $id: String!",
                ", filter: { name: { containsIgnoreCase: $term } }",
            ),
            json!({"term":term,"id":team}),
        )
    }
}

pub fn limit(prefs: &Value, name: &str, default: usize, max: usize) -> usize {
    let value = prefs[name]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .or_else(|| prefs[name].as_str()?.trim().parse().ok())
        .filter(|number| *number > 0)
        .unwrap_or(default);
    value.min(max)
}

pub fn choices(rows: &[Value], label: &str) -> Vec<Choice> {
    rows.iter()
        .filter_map(|row| {
            let id = row["id"].as_str()?;
            let title = row[label]
                .as_str()
                .filter(|text| !text.is_empty())
                .unwrap_or(id);
            Some(Choice::new(id, title))
        })
        .collect()
}

pub async fn teams(api: &Api<'_>) -> Result<Vec<Value>, String> {
    api.collect(
        &Connection::root("Teams", "teams", api::TEAMS),
        json!({}),
        350,
    )
    .await
}

pub async fn viewer(api: &Api<'_>) -> Result<Value, String> {
    let value = api
        .request("query Viewer { viewer { id displayName } }", json!({}))
        .await?;
    if value["viewer"]["id"].as_str().is_none() {
        return Err("Linear returned no authenticated user.".into());
    }
    Ok(value["viewer"].clone())
}

pub async fn states(api: &Api<'_>, team: &str) -> Result<Vec<Value>, String> {
    let spec = Connection::root("TeamStates", "states", api::STATUSES)
        .nested("team(id: $id)", "team.states")
        .args(", $id: String!", "");
    let mut rows = api.collect(&spec, json!({"id":team}), 100).await?;
    rows.sort_by(|a, b| {
        a["position"]
            .as_f64()
            .unwrap_or_default()
            .total_cmp(&b["position"].as_f64().unwrap_or_default())
    });
    Ok(rows)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Form {
    pub kind: FormKind,
    pub id: String,
    pub context: Value,
    pub values: Value,
    pub options: HashMap<String, Vec<Choice>>,
}

impl Form {
    pub fn new(kind: FormKind, editing: Value, context: Value) -> Self {
        let id = string(&editing, "id");
        let mut values = json!({});
        match kind {
            FormKind::Issue => {
                for key in ["title", "description", "priority", "estimate", "dueDate"] {
                    values[key] = editing[key].clone();
                }
                for (key, entity) in [
                    ("teamId", "team"),
                    ("stateId", "state"),
                    ("assigneeId", "assignee"),
                    ("cycleId", "cycle"),
                    ("projectId", "project"),
                    ("projectMilestoneId", "projectMilestone"),
                ] {
                    values[key] = editing[entity]["id"].clone();
                    if values[key].is_null() {
                        values[key] = context[key].clone();
                    }
                }
                values["labelIds"] = if editing["labelIds"].is_array() {
                    editing["labelIds"].clone()
                } else {
                    ids(&editing["labels"])
                };
            }
            FormKind::Project => {
                for key in ["name", "description", "startDate", "targetDate"] {
                    values[key] = editing[key].clone();
                }
                values["teamIds"] = ids(&editing["teams"]);
                values["memberIds"] = ids(&editing["members"]);
                values["leadId"] = editing["lead"]["id"].clone();
                values["statusId"] = editing["status"]["id"].clone();
            }
            FormKind::Document => {
                values["title"] = editing["title"].clone();
                values["content"] = editing["content"].clone();
            }
            FormKind::Parent => values["identifier"] = editing["parent"]["identifier"].clone(),
            _ => {}
        }
        Self {
            kind,
            id,
            context,
            values,
            options: HashMap::new(),
        }
    }

    pub async fn load(&mut self, api: &Api<'_>, prefs: &Value) -> Result<(), String> {
        match self.kind {
            FormKind::Issue | FormKind::Project => {
                let people = Connection::root("Users", "users", api::PEOPLE)
                    .args("", ", filter: { active: { eq: true } }");
                let (teams, users) =
                    tokio::try_join!(teams(api), api.collect(&people, json!({}), 350))?;
                let team_choices = choices(&teams, "name");
                let people_choices = choices(&users, "displayName");
                if self.kind == FormKind::Project {
                    let statuses =
                        Connection::root("ProjectStatuses", "projectStatuses", "id name type");
                    self.options.insert(
                        "statusId".into(),
                        choices(&api.collect(&statuses, json!({}), 100).await?, "name"),
                    );
                    if self.values["teamIds"] == json!([])
                        && self.context["teamId"]
                            .as_str()
                            .is_some_and(|id| !id.is_empty())
                    {
                        self.values["teamIds"] = json!([self.context["teamId"]]);
                    }
                    self.options.insert("teamIds".into(), team_choices);
                    self.options.insert("leadId".into(), people_choices.clone());
                    self.options.insert("memberIds".into(), people_choices);
                } else {
                    self.options.insert("teamId".into(), team_choices);
                    self.options.insert("assigneeId".into(), people_choices);
                    if self.values["teamId"].is_null() {
                        self.values["teamId"] = teams
                            .first()
                            .map(|team| team["id"].clone())
                            .unwrap_or_default();
                    }
                    self.load_team(api, prefs).await?;
                }
            }
            _ => {}
        }
        self.preserve_selected_options();
        Ok(())
    }

    fn preserve_selected_options(&mut self) {
        for (key, options) in &mut self.options {
            let value = &self.values[key];
            let selected: Vec<&str> = if let Some(list) = value.as_array() {
                list.iter().filter_map(Value::as_str).collect()
            } else {
                value.as_str().into_iter().collect()
            };
            for id in selected.into_iter().filter(|id| !id.is_empty()) {
                if !options.iter().any(|option| option.id == id) {
                    options.push(Choice::new(id, id));
                }
            }
        }
    }

    pub async fn load_team(&mut self, api: &Api<'_>, prefs: &Value) -> Result<(), String> {
        let id = string(&self.values, "teamId");
        if id.is_empty() {
            return Ok(());
        }
        let labels = Connection::root("TeamLabels", "labels", "id name")
            .nested("team(id: $id)", "team.labels")
            .args(", $id: String!", "");
        let cycles = Connection::root("TeamCycles", "cycles", "id name number")
            .nested("team(id: $id)", "team.cycles")
            .args(", $id: String!", ", orderBy: updatedAt");
        let (projects, project_args) = projects("", &id);
        let (states, labels, cycles, projects) = tokio::try_join!(
            states(api, &id),
            api.collect(
                &labels,
                json!({"id":id}),
                limit(prefs, "labelsLimit", 100, 350)
            ),
            api.collect(&cycles, json!({"id":id}), 100),
            api.collect(&projects, project_args, 200)
        )?;
        self.options
            .insert("stateId".into(), choices(&states, "name"));
        self.options
            .insert("labelIds".into(), choices(&labels, "name"));
        self.options.insert(
            "cycleId".into(),
            cycles
                .iter()
                .filter_map(|cycle| {
                    Some(Choice::new(
                        cycle["id"].as_str()?,
                        cycle["name"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("Cycle {}", cycle["number"])),
                    ))
                })
                .collect(),
        );
        self.options
            .insert("projectId".into(), choices(&projects, "name"));
        self.load_milestones(api).await
    }

    pub async fn load_milestones(&mut self, api: &Api<'_>) -> Result<(), String> {
        let id = string(&self.values, "projectId");
        let milestones = if id.is_empty() {
            Vec::new()
        } else {
            api.collect(
                &project_connection(
                    "Milestones",
                    "projectMilestones",
                    "project.projectMilestones",
                    api::MILESTONE,
                ),
                json!({"id":id}),
                150,
            )
            .await?
        };
        self.options
            .insert("projectMilestoneId".into(), choices(&milestones, "name"));
        Ok(())
    }
}

fn ids(connection: &Value) -> Value {
    json!(connection["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str())
        .collect::<Vec<_>>())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Screen {
    List(Listing),
    Detail(Entity, Value),
    Form(Form),
    Result(Entity, Value, String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Action {
    Settings,
    Open(String),
    Copy(String),
    Back,
    Refresh,
    More,
    Select,
    Detail(Entity, Value),
    List(Kind, String),
    Form(FormKind, Value, Value),
    Submit,
    FormChange(String),
    AssignSelf(String),
    Subscribe(String, bool),
    Archive(Entity, String),
    Notification(String, NoteAction),
    OpenInbox,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum NoteAction {
    Open,
    Read,
    Unread,
    Archive,
    Snooze,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub screen: Screen,
    pub history: Vec<Screen>,
    pub actions: HashMap<String, Action>,
    pub cycle_teams: Vec<Value>,
    pub revision: u64,
    pub notice: String,
}

impl Session {
    pub fn new(screen: Screen) -> Self {
        Self {
            screen,
            history: Vec::new(),
            actions: HashMap::new(),
            cycle_teams: Vec::new(),
            revision: 0,
            notice: String::new(),
        }
    }

    pub fn push(&mut self, screen: Screen) -> Result<(), String> {
        if self.history.len() >= 16 {
            return Err("Too many open Linear views. Go back to continue.".into());
        }
        self.history
            .push(std::mem::replace(&mut self.screen, screen));
        Ok(())
    }
}
