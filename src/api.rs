use reqwest::{
    header::{HeaderValue, AUTHORIZATION},
    redirect::Policy,
    Client, Url,
};
use serde_json::{json, Value};
use std::{collections::HashSet, time::Duration};

pub const ISSUE: &str = "id identifier title branchName priority priorityLabel estimate dueDate updatedAt url archivedAt state { id type name color } assignee { id displayName } team { id name key issueEstimationType activeCycle { id } } cycle { id name number } parent { id identifier title } project { id name } projectMilestone { id name } labels { nodes { id name } }";
pub const PROJECT: &str = "id name description progress url startDate targetDate status { id name type } lead { id displayName } members { nodes { id displayName } } teams { nodes { id key name } }";
pub const DOCUMENT: &str = "id title url createdAt updatedAt sortOrder project { id name } initiative { id name } creator { displayName }";
pub const FAVORITE: &str = "id type url updatedAt customView { id name } cycle { id name number team { key } } document { id title slugId } issue { id title url state { name } } label { id name } project { id name url } initiative { id name } user { id displayName name url }";
pub const NOTIFICATION: &str = "id type createdAt readAt snoozedUntilAt title subtitle url actor { displayName } botActor { name }";
pub const TEAMS: &str = "id name key activeCycle { id name number }";
pub const PEOPLE: &str = "id name displayName";
pub const STATUSES: &str = "id name type position";
pub const COMMENT: &str = "id body createdAt url user { displayName }";
pub const ATTACHMENT: &str = "id title subtitle url";
pub const MILESTONE: &str = "id name targetDate";
pub const UPDATE: &str = "id body url health createdAt user { displayName }";

const MAX_RESPONSE: usize = 4 * 1024 * 1024;

pub fn string(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}

#[derive(Clone)]
pub struct Transport {
    client: Client,
    endpoint: Url,
}

impl Transport {
    pub fn new() -> Result<Self, String> {
        #[cfg(not(feature = "fixture"))]
        let endpoint = Url::parse("https://api.linear.app/graphql").expect("constant endpoint");
        #[cfg(feature = "fixture")]
        let endpoint = match std::env::var("TEZBAR_LINEAR_TEST_ENDPOINT") {
            Ok(value) => {
                let url = Url::parse(&value).map_err(|_| "Invalid test endpoint")?;
                if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") {
                    return Err("Test endpoint must be HTTP on 127.0.0.1".into());
                }
                url
            }
            Err(_) => Url::parse("http://127.0.0.1:9/graphql").expect("constant fixture endpoint"),
        };
        let client = Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(12))
            .connect_timeout(Duration::from_secs(4))
            .user_agent("tezbar-linear/0.1.0");
        #[cfg(feature = "fixture")]
        let client = if endpoint.scheme() == "http" {
            client.no_proxy()
        } else {
            client
        };
        Ok(Self {
            client: client
                .build()
                .map_err(|_| "Unable to initialize Linear HTTP client")?,
            endpoint,
        })
    }

    pub fn authenticate(&self, key: &str) -> Result<Api<'_>, String> {
        if key.is_empty() {
            return Err("Add a Linear personal API key in Tezbar extension settings.".into());
        }
        if key.len() > 512 || key.bytes().any(|c| !c.is_ascii_graphic()) {
            return Err("The Linear API key has an invalid format.".into());
        }
        let mut header = HeaderValue::from_str(key).map_err(|_| "Invalid Linear API key")?;
        header.set_sensitive(true);
        Ok(Api {
            transport: self,
            header,
        })
    }
}

pub struct Api<'a> {
    transport: &'a Transport,
    header: HeaderValue,
}

pub struct Page {
    pub rows: Vec<Value>,
    pub next: Option<String>,
    pub organization: String,
}

#[derive(Clone)]
pub struct Connection {
    pub name: &'static str,
    pub field: &'static str,
    pub args: &'static str,
    pub fields: &'static str,
    pub wrap: &'static str,
    pub vars: &'static str,
    pub path: &'static str,
    pub organization: bool,
}

impl Connection {
    pub fn root(name: &'static str, field: &'static str, fields: &'static str) -> Self {
        Self {
            name,
            field,
            fields,
            args: "",
            wrap: "",
            vars: "",
            path: field,
            organization: false,
        }
    }

    pub fn args(mut self, vars: &'static str, args: &'static str) -> Self {
        self.vars = vars;
        self.args = args;
        self
    }

    pub fn nested(mut self, wrap: &'static str, path: &'static str) -> Self {
        self.wrap = wrap;
        self.path = path;
        self
    }

    pub fn organization(mut self) -> Self {
        self.organization = true;
        self
    }

    fn query(&self) -> String {
        let connection = format!(
            "{}(first: $first, after: $after{}) {{ nodes {{ {} }} pageInfo {{ hasNextPage endCursor }} }}",
            self.field, self.args, self.fields,
        );
        let body = if self.wrap.is_empty() {
            connection
        } else {
            format!("{} {{ {} }}", self.wrap, connection)
        };
        let organization = if self.organization {
            " organization { urlKey }"
        } else {
            ""
        };
        format!(
            "query {}($first: Int!, $after: String{}) {{ {}{} }}",
            self.name, self.vars, body, organization,
        )
    }
}

impl Api<'_> {
    pub async fn request(&self, query: &str, variables: Value) -> Result<Value, String> {
        let mut reply = self
            .transport
            .client
            .post(self.transport.endpoint.clone())
            .header(AUTHORIZATION, self.header.clone())
            .header("public-file-urls-expire-in", "60")
            .json(&json!({"query":query,"variables":variables}))
            .send()
            .await
            .map_err(|_| "Could not reach Linear. Check your connection and try again.")?;

        let status = reply.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err("Linear rejected the API key or its permissions.".into());
        }
        if status.as_u16() == 429 {
            return Err("Linear rate limit reached. Try again later.".into());
        }
        if !status.is_success() {
            return Err(format!("Linear returned HTTP {}.", status.as_u16()));
        }

        let mut body = Vec::new();
        while let Some(chunk) = reply
            .chunk()
            .await
            .map_err(|_| "Could not read the Linear response.")?
        {
            if body.len() + chunk.len() > MAX_RESPONSE {
                return Err("The Linear response is too large. Reduce the API issue limit.".into());
            }
            body.extend_from_slice(&chunk);
        }
        let mut envelope: Value =
            serde_json::from_slice(&body).map_err(|_| "Invalid response from Linear.")?;
        if let Some(errors) = envelope["errors"].as_array().filter(|v| !v.is_empty()) {
            // Linear's messages can echo untrusted input. Never display arbitrary reply text.
            let code = errors[0]["extensions"]["code"].as_str().unwrap_or_default();
            return Err(match code {
                "RATELIMITED" => "Linear rate limit reached. Try again later.",
                "FORBIDDEN" | "AUTHENTICATION_ERROR" => {
                    "Linear rejected the API key or its permissions."
                }
                _ => {
                    "Linear rejected the request. Check the selected values or API key permissions."
                }
            }
            .into());
        }
        let data = envelope["data"].take();
        if !data.is_object() {
            return Err("Linear returned no data.".into());
        }
        Ok(data)
    }

    pub async fn page(
        &self,
        spec: &Connection,
        mut variables: Value,
        first: usize,
        after: Option<&str>,
    ) -> Result<Page, String> {
        variables["first"] = json!(first);
        variables["after"] = json!(after);
        let mut data = self.request(&spec.query(), variables).await?;
        let organization = string(&data["organization"], "urlKey");
        let mut connection = &mut data;
        for key in spec.path.split('.') {
            connection = connection
                .get_mut(key)
                .ok_or("Linear returned an incomplete page.")?;
        }
        // A null connection is valid when a custom view has no issue list.
        if connection.is_null() && spec.path == "customView.issues" {
            return Ok(Page {
                rows: Vec::new(),
                next: None,
                organization,
            });
        }
        let rows = connection["nodes"]
            .take()
            .as_array()
            .cloned()
            .ok_or("Linear returned an invalid page.")?;
        let more = connection["pageInfo"]["hasNextPage"]
            .as_bool()
            .ok_or("Linear returned no pagination information.")?;
        let next = if more {
            Some(
                connection["pageInfo"]["endCursor"]
                    .as_str()
                    .filter(|value| {
                        !value.is_empty() && value.len() <= 4096 && Some(*value) != after
                    })
                    .ok_or("Linear returned an invalid pagination cursor.")?
                    .to_owned(),
            )
        } else {
            None
        };
        Ok(Page {
            rows,
            next,
            organization,
        })
    }

    pub async fn collect(
        &self,
        spec: &Connection,
        variables: Value,
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        let mut rows = Vec::new();
        let mut after = None;
        let mut seen = HashSet::new();
        while rows.len() < limit {
            let page = self
                .page(
                    spec,
                    variables.clone(),
                    (limit - rows.len()).min(50),
                    after.as_deref(),
                )
                .await?;
            rows.extend(page.rows);
            let Some(next) = page.next else { break };
            if !seen.insert(next.clone()) || seen.len() > 30 {
                return Err("Linear pagination did not terminate.".into());
            }
            after = Some(next);
        }
        rows.truncate(limit);
        Ok(rows)
    }

    pub async fn entity(
        &self,
        name: &str,
        field: &str,
        id: &str,
        fields: &str,
    ) -> Result<Value, String> {
        let query = format!("query {name}($id: String!) {{ {field}(id: $id) {{ {fields} }} }}");
        let mut data = self.request(&query, json!({"id":id})).await?;
        if data[field].is_object() {
            Ok(data[field].take())
        } else {
            Err("Linear could not find this item.".into())
        }
    }

    pub async fn mutation(
        &self,
        operation: Mutation,
        id: Option<&str>,
        input: Option<Value>,
    ) -> Result<Value, String> {
        let (field, input_type, response) = operation.spec();
        let mut variables = json!({});
        let mut definitions = Vec::new();
        let mut arguments = Vec::new();
        if let Some(id) = id.filter(|id| !id.is_empty()) {
            definitions.push("$id: String!".into());
            arguments.push("id: $id");
            variables["id"] = json!(id);
        }
        if let (Some(input), Some(kind)) = (input, input_type) {
            definitions.push(format!("$input: {kind}!"));
            arguments.push("input: $input");
            variables["input"] = input;
        }
        let query = format!(
            "mutation {}({}) {{ {field}({}) {{ success {response} }} }}",
            operation.name(),
            definitions.join(", "),
            arguments.join(", ")
        );
        let mut data = self.request(&query, variables).await?;
        if data[field]["success"] != true {
            return Err("Linear did not confirm the change.".into());
        }
        Ok(data[field].take())
    }
}

#[derive(Clone, Copy)]
pub enum Mutation {
    IssueCreate,
    IssueUpdate,
    IssueArchive,
    CommentCreate,
    ProjectCreate,
    ProjectUpdate,
    ProjectArchive,
    MilestoneCreate,
    AttachmentCreate,
    NotificationUpdate,
    NotificationArchive,
    DocumentCreate,
    DocumentUpdate,
}

impl Mutation {
    fn name(self) -> &'static str {
        match self {
            Self::IssueCreate => "IssueCreate",
            Self::IssueUpdate => "IssueUpdate",
            Self::IssueArchive => "IssueArchive",
            Self::CommentCreate => "CommentCreate",
            Self::ProjectCreate => "ProjectCreate",
            Self::ProjectUpdate => "ProjectUpdate",
            Self::ProjectArchive => "ProjectArchive",
            Self::MilestoneCreate => "MilestoneCreate",
            Self::AttachmentCreate => "AttachmentCreate",
            Self::NotificationUpdate => "NotificationUpdate",
            Self::NotificationArchive => "NotificationArchive",
            Self::DocumentCreate => "DocumentCreate",
            Self::DocumentUpdate => "DocumentUpdate",
        }
    }

    fn spec(self) -> (&'static str, Option<&'static str>, &'static str) {
        match self {
            Self::IssueCreate => (
                "issueCreate",
                Some("IssueCreateInput"),
                "issue { id identifier title url branchName }",
            ),
            Self::IssueUpdate => (
                "issueUpdate",
                Some("IssueUpdateInput"),
                "issue { id identifier title url branchName }",
            ),
            Self::IssueArchive => ("issueArchive", None, ""),
            Self::CommentCreate => (
                "commentCreate",
                Some("CommentCreateInput"),
                "comment { id url }",
            ),
            Self::ProjectCreate => (
                "projectCreate",
                Some("ProjectCreateInput"),
                "project { id name url }",
            ),
            Self::ProjectUpdate => (
                "projectUpdate",
                Some("ProjectUpdateInput"),
                "project { id name url }",
            ),
            Self::ProjectArchive => ("projectArchive", None, ""),
            Self::MilestoneCreate => (
                "projectMilestoneCreate",
                Some("ProjectMilestoneCreateInput"),
                "",
            ),
            Self::AttachmentCreate => ("attachmentCreate", Some("AttachmentCreateInput"), ""),
            Self::NotificationUpdate => ("notificationUpdate", Some("NotificationUpdateInput"), ""),
            Self::NotificationArchive => ("notificationArchive", None, ""),
            Self::DocumentCreate => (
                "documentCreate",
                Some("DocumentCreateInput"),
                "document { id title url }",
            ),
            Self::DocumentUpdate => (
                "documentUpdate",
                Some("DocumentUpdateInput"),
                "document { id title url }",
            ),
        }
    }
}
