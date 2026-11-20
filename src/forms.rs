use crate::{
    api::{string, Api, Mutation},
    model::{self, Entity, Form, FormKind},
};
use reqwest::Url;
use serde_json::{json, Value};
use tezbar_extension_sdk::ExtensionContext;
use time::{format_description::well_known::Rfc3339, Duration, OffsetDateTime};

pub fn web_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "Enter a valid web URL.")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Enter an HTTP or HTTPS URL without embedded credentials.".into());
    }
    Ok(url)
}

pub async fn open(ctx: &ExtensionContext, raw: &str) -> Result<(), String> {
    let target = web_url(raw)?;
    ctx.host("host.shell.open", json!({"target":target.as_str()}))
        .await
        .map(|_| ())
}

pub async fn settings(ctx: &ExtensionContext) -> Result<(), String> {
    ctx.host(
        "host.settings.open",
        json!({"section":"settings-extensions"}),
    )
    .await
    .map(|_| ())
}

pub async fn close(ctx: &ExtensionContext, prefs: &Value) -> Result<(), String> {
    if prefs["shouldCloseMainWindow"] == true {
        ctx.host("host.window.visibility", json!({"visible":false}))
            .await?;
    }
    Ok(())
}

pub fn required(values: &Value, field: &str, label: &str, max: usize) -> Result<String, String> {
    let value = string(values, field);
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{label} is required."));
    }
    if value.len() > max {
        return Err(format!("{label} exceeds the supported length."));
    }
    Ok(value.into())
}

fn optional_text(values: &Value, field: &str, max: usize) -> Result<Value, String> {
    let value = values[field].as_str().unwrap_or_default();
    if value.len() > max {
        return Err(format!("{field} exceeds the supported length."));
    }
    Ok(json!(value))
}

fn date(value: &Value) -> Result<Value, String> {
    let Some(raw) = value.as_str().filter(|text| !text.is_empty()) else {
        return Ok(Value::Null);
    };
    let format =
        time::format_description::parse("[year]-[month]-[day]").expect("static date format");
    if let Some(prefix) = raw
        .get(..10)
        .filter(|_| raw.len() == 10 || raw.as_bytes().get(10) == Some(&b'T'))
    {
        if time::Date::parse(prefix, &format).is_ok() {
            return Ok(json!(prefix));
        }
    }
    Err("Enter a valid calendar date.".into())
}

pub fn now(hours: i64) -> Value {
    json!((OffsetDateTime::now_utc() + Duration::hours(hours))
        .format(&Rfc3339)
        .expect("UTC timestamp"))
}

async fn resolve_issue(api: &Api<'_>, identifier: &str) -> Result<String, String> {
    let item = api
        .entity("ResolveIssue", "issue", identifier, "id")
        .await?;
    required(&item, "id", "Issue ID", 256)
}

pub struct Saved {
    pub kind: Entity,
    pub entity: Value,
    pub message: &'static str,
}

impl Form {
    pub fn editing(&self) -> bool {
        !self.id.is_empty()
            && matches!(
                self.kind,
                FormKind::Issue | FormKind::Project | FormKind::Document
            )
    }

    pub fn apply(&mut self, incoming: &Value) {
        if let Some(values) = incoming.as_object() {
            for (key, value) in values {
                if !key.starts_with('_') {
                    self.values[key] = value.clone();
                }
            }
        }
    }

    pub async fn submit(&self, api: &Api<'_>) -> Result<Saved, String> {
        let editing = self.editing();
        let id = editing.then_some(self.id.as_str());
        let values = &self.values;
        let context = &self.context;
        let (mutation, data, kind, response, message) = match self.kind {
            FormKind::Issue => {
                let team = required(values, "teamId", "Team", 256)?;
                check_option(self, "teamId", &team)?;
                let title = required(values, "title", "Title", 1024)?;
                let mut data = json!({"teamId":team,"title":title,"description":optional_text(values,"description",160_000)?});
                for key in [
                    "stateId",
                    "assigneeId",
                    "cycleId",
                    "projectId",
                    "projectMilestoneId",
                ] {
                    apply_option(self, &mut data, key, editing && key != "stateId")?;
                }
                let labels = checked_tags(self, "labelIds")?;
                data["labelIds"] = json!(labels);
                let priority = values["priority"]
                    .as_i64()
                    .or_else(|| values["priority"].as_str()?.parse().ok())
                    .unwrap_or(0);
                if !(0..=4).contains(&priority) {
                    return Err("Select a valid priority.".into());
                }
                data["priority"] = json!(priority);
                let estimate = values["estimate"]
                    .as_i64()
                    .or_else(|| values["estimate"].as_str()?.trim().parse().ok());
                if let Some(number) = estimate {
                    if !(0..=1000).contains(&number) {
                        return Err("Enter a valid estimate.".into());
                    }
                    data["estimate"] = json!(number);
                } else if values["estimate"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty())
                {
                    return Err("Enter a whole number for the estimate.".into());
                } else if editing {
                    data["estimate"] = Value::Null;
                }
                apply_date(values, &mut data, "dueDate", editing)?;
                if let Some(parent) = context["parentId"].as_str() {
                    data["parentId"] = json!(parent);
                }
                if editing {
                    (
                        Mutation::IssueUpdate,
                        data,
                        Entity::Issue,
                        "issue",
                        "Issue updated",
                    )
                } else {
                    (
                        Mutation::IssueCreate,
                        data,
                        Entity::Issue,
                        "issue",
                        "Issue created",
                    )
                }
            }
            FormKind::Project => {
                let teams = checked_tags(self, "teamIds")?;
                if teams.is_empty() {
                    return Err("Select at least one team.".into());
                }
                let mut data = json!({"teamIds":teams,"name":required(values,"name","Name",1024)?,
                    "description":optional_text(values,"description",160_000)?,"memberIds":checked_tags(self,"memberIds")?});
                apply_option(self, &mut data, "statusId", false)?;
                apply_option(self, &mut data, "leadId", editing)?;
                for key in ["startDate", "targetDate"] {
                    apply_date(values, &mut data, key, editing)?;
                }
                if editing {
                    (
                        Mutation::ProjectUpdate,
                        data,
                        Entity::Project,
                        "project",
                        "Project updated",
                    )
                } else {
                    (
                        Mutation::ProjectCreate,
                        data,
                        Entity::Project,
                        "project",
                        "Project created",
                    )
                }
            }
            FormKind::Comment => (
                Mutation::CommentCreate,
                json!({
                    "issueId":required(context,"issueId","Issue ID",256)?,"body":required(values,"body","Comment",160_000)?,
                }),
                Entity::Text,
                "comment",
                "Comment added",
            ),
            FormKind::Milestone => {
                let mut data = json!({"projectId":required(context,"projectId","Project ID",256)?,
                    "name":required(values,"name","Milestone name",1024)?,
                    "description":optional_text(values,"description",160_000)?});
                apply_date(values, &mut data, "targetDate", false)?;
                (
                    Mutation::MilestoneCreate,
                    data,
                    Entity::Text,
                    "",
                    "Milestone created",
                )
            }
            FormKind::Attachment => {
                let url = web_url(&required(values, "url", "URL", 4096)?)?;
                (
                    Mutation::AttachmentCreate,
                    json!({"issueId":required(context,"issueId","Issue ID",256)?,
                    "url":url.as_str(),"title":required(values,"title","Title",1024)?,
                    "subtitle":optional_text(values,"subtitle",1024)?}),
                    Entity::Text,
                    "",
                    "Link attached",
                )
            }
            FormKind::Document => {
                let mut data = json!({"title":required(values,"title","Title",1024)?,"content":optional_text(values,"content",160_000)?});
                if editing {
                    (
                        Mutation::DocumentUpdate,
                        data,
                        Entity::Document,
                        "document",
                        "Document updated",
                    )
                } else {
                    data["projectId"] = json!(required(context, "projectId", "Project ID", 256)?);
                    (
                        Mutation::DocumentCreate,
                        data,
                        Entity::Document,
                        "document",
                        "Document created",
                    )
                }
            }
            FormKind::Parent => {
                let parent = string(values, "identifier");
                let parent = if parent.trim().is_empty() {
                    Value::Null
                } else {
                    json!(resolve_issue(api, parent.trim()).await?)
                };
                if parent == self.id {
                    return Err("An issue cannot be its own parent.".into());
                }
                let update = api
                    .mutation(
                        Mutation::IssueUpdate,
                        Some(&self.id),
                        Some(json!({"parentId":parent})),
                    )
                    .await?;
                return Ok(Saved {
                    kind: Entity::Issue,
                    entity: update["issue"].clone(),
                    message: "Parent issue changed",
                });
            }
        };
        let updated = api.mutation(mutation, id, Some(data)).await?;
        Ok(Saved {
            kind,
            entity: updated[response].clone(),
            message,
        })
    }
}

fn check_option(form: &Form, key: &str, id: &str) -> Result<(), String> {
    if form
        .options
        .get(key)
        .is_some_and(|values| values.iter().any(|option| option.id == id))
    {
        Ok(())
    } else {
        Err(format!("Select a valid value for {key}."))
    }
}

fn apply_option(form: &Form, output: &mut Value, key: &str, clear: bool) -> Result<(), String> {
    let selected = string(&form.values, key);
    if !selected.is_empty() {
        check_option(form, key, &selected)?;
        output[key] = json!(selected);
    } else if clear {
        output[key] = Value::Null;
    }
    Ok(())
}

fn checked_tags(form: &Form, key: &str) -> Result<Vec<String>, String> {
    let mut selected = Vec::new();
    for value in form.values[key].as_array().into_iter().flatten() {
        let id = value.as_str().ok_or("Invalid form option.")?;
        check_option(form, key, id)?;
        if !selected.iter().any(|existing| existing == id) {
            selected.push(id.into());
        }
    }
    Ok(selected)
}

fn apply_date(values: &Value, output: &mut Value, key: &str, clear: bool) -> Result<(), String> {
    let value = date(&values[key])?;
    if clear || !value.is_null() {
        output[key] = value;
    }
    Ok(())
}

pub async fn quick_issue(
    api: &Api<'_>,
    ctx: &ExtensionContext,
    prefs: &Value,
    args: &Value,
) -> Result<Value, String> {
    let title = required(args, "title", "Title", 1024)?;
    let description = optional_text(args, "description", 160_000)?;
    close(ctx, prefs).await?;
    let (viewer, teams) = tokio::try_join!(model::viewer(api), model::teams(api))?;
    let preferred = string(prefs, "preferredTeamKey");
    let team = if preferred.trim().is_empty() {
        teams.first()
    } else {
        teams
            .iter()
            .find(|team| string(team, "key").eq_ignore_ascii_case(preferred.trim()))
    }
    .ok_or(
        "The preferred team is unavailable. Set a valid Linear team key in command preferences.",
    )?;
    let team = required(team, "id", "Team ID", 256)?;
    let mut data =
        json!({"title":title,"description":description,"teamId":team,"assigneeId":viewer["id"]});
    let preferred = string(prefs, "preferredStatusName");
    if !preferred.trim().is_empty() {
        let states = model::states(api, &team).await?;
        let state = states
            .iter()
            .find(|state| string(state, "name").eq_ignore_ascii_case(preferred.trim()))
            .ok_or("The preferred status is unavailable in the selected team.")?;
        data["stateId"] = state["id"].clone();
    }
    let result = api
        .mutation(Mutation::IssueCreate, None, Some(data))
        .await?;
    Ok(tezbar_extension_sdk::no_view(&format!(
        "Created {}",
        string(&result["issue"], "identifier")
    )))
}

pub async fn quick_comment(
    api: &Api<'_>,
    ctx: &ExtensionContext,
    prefs: &Value,
    args: &Value,
) -> Result<Value, String> {
    let identifier = required(args, "issueId", "Issue ID", 256)?;
    let body = required(args, "comment", "Comment", 160_000)?;
    close(ctx, prefs).await?;
    let id = resolve_issue(api, &identifier).await?;
    api.mutation(
        Mutation::CommentCreate,
        None,
        Some(json!({"issueId":id,"body":body})),
    )
    .await?;
    Ok(tezbar_extension_sdk::no_view("Comment added"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_validated_and_converted_to_linear_dates() {
        assert_eq!(date(&json!("2024-02-29T00:00:00Z")).unwrap(), "2024-02-29");
        assert!(date(&json!("2023-02-29")).is_err());
        assert!(date(&json!("2025-02-02bad")).is_err());
    }

    #[test]
    fn host_open_only_accepts_web_urls_without_credentials() {
        for raw in [
            "file:///private",
            "javascript:test()",
            "https://name:secret@example.com",
        ] {
            assert!(web_url(raw).is_err());
        }
        assert!(web_url("https://linear.app/test/issue/ABC-1").is_ok());
    }
}
