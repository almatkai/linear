#!/usr/bin/env python3
"""Exercise the genuine Linear process protocol with an isolated HTTP fixture."""
import argparse
import copy
import json
import os
import queue
import re
import subprocess
import tempfile
import threading
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = json.loads((ROOT / "extension.json").read_text(encoding="utf-8"))
COMMANDS = [item["name"] for item in MANIFEST["commands"]]
TEST_KEY = "lin_api_synthetic_local_fixture"
ISSUE = {
    "id": "issue-1", "identifier": "SYN-1", "title": "Fixture issue",
    "description": "Fixture issue body", "url": "https://linear.app/synthetic/issue/SYN-1",
    "branchName": "synthetic/SYN-1", "priority": 2, "priorityLabel": "High",
    "team": {"id": "team-1", "name": "Synthetic", "key": "SYN"},
    "state": {"id": "status-1", "name": "Todo", "type": "unstarted"},
    "labels": {"nodes": [{"id": "label-1", "name": "Regression"}]},
}
TEAMS = [
    {"id": "team-1", "name": "Synthetic", "key": "SYN", "activeCycle": {"id": "cycle-1", "number": 5}},
    {"id": "team-2", "name": "Second", "key": "TWO", "activeCycle": {"id": "cycle-2", "number": 6}},
]
PROJECT = {"id": "project-1", "name": "Synthetic Project", "description": "Project description",
           "status": {"id": "project-status", "name": "Planned"}, "url": "https://linear.app/synthetic/project/test",
           "teams": {"nodes": TEAMS[:1]}, "members": {"nodes": []}}
DOCUMENT = {"id": "document-1", "title": "Test document", "content": "# Synthetic Document",
            "url": "https://linear.app/synthetic/document/test"}
NOTE = {"id": "notification-1", "title": "Fixture notification", "subtitle": "Comment on SYN-1",
        "url": "/synthetic/issue/SYN-1", "issue": ISSUE}
CUSTOM_VIEW = {"id": "view-1", "name": "Fixture view", "modelName": "Issue"}
ACTIVE = {"id": "user-1", "displayName": "Fixture User"}
ORGANIZATION = {"urlKey": "synthetic"}


def page(rows, cursor=None):
    return {"nodes": rows, "pageInfo": {"hasNextPage": cursor is not None, "endCursor": cursor}}


class Fixture:
    def __init__(self):
        self.requests = []
        self.issues = copy.deepcopy(ISSUE)
        self.notifications = [copy.deepcopy(NOTE)]
        self.grouped = [{"id": "group-1", "title": "Fixture group", "subtitle": "Issue activity", "url": NOTE["url"]}]
        self.mutations = []
        self.invalid_cursors = False

    def request(self, query, v):
        name = re.match(r"^(query|mutation) (\w+)", query).group(2)
        self.requests.append((name, v))
        if name == "SearchIssues" and v["term"] == "echo error":
            return {"errors": [{"message": f"must never show {TEST_KEY}"}]}
        if name in {"Issues", "SearchIssues"}:
            root = "searchIssues" if name == "SearchIssues" else "issues"
            if v["after"]:
                second = dict(self.issues, id="issue-2", identifier="SYN-2", title="Second fixture issue")
                cursor = v["after"] if self.invalid_cursors else None
                return {"data": {root: page([second], cursor)}}
            return {"data": {root: page([self.issues], "issues-page-2")}}
        bodies = {
            "Viewer": {"viewer": ACTIVE},
            "Teams": {"teams": page(TEAMS)},
            "AssignedIssues": {"viewer": {"assignedIssues": page([self.issues])}},
            "CreatedIssues": {"viewer": {"createdIssues": page([self.issues])}},
            "CycleIssues": {"cycle": {"issues": page([self.issues])}},
            "ViewIssues": {"customView": {"issues": page([self.issues])}},
            "SubIssues": {"issue": {"children": page([self.issues])}},
            "CustomViews": {"customViews": page([CUSTOM_VIEW, {"id": "not-an-issue-view", "modelName": "Project"}]), "organization": ORGANIZATION},
            "Favorites": {"favorites": page([{"id": "favorite-1", "type": "issue", "issue": self.issues}]), "organization": ORGANIZATION},
            "Notifications": {"notifications": page(self.notifications), "organization": ORGANIZATION},
            "UnreadInbox": {"inboxNotifications": page(self.grouped), "organization": ORGANIZATION},
            "Issue": {"issue": self.issues},
            "ResolveIssue": {"issue": {"id": "issue-1"}},
            "Projects": {"team": {"projects": page([PROJECT])}} if "team(id:" in query else {"projects": page([PROJECT])},
            "Project": {"project": PROJECT},
            "ProjectUpdates": {"project": {"projectUpdates": page([{"id": "update-1", "health": "onTrack", "body": "Update body"}])}},
            "Milestones": {"project": {"projectMilestones": page([{"id": "milestone-1", "name": "First milestone"}])}},
            "Documents": {"documents": page([DOCUMENT])},
            "Document": {"documents": {"nodes": [DOCUMENT]}},
            "IssueComments": {"issue": {"comments": page([{"id": "comment-1", "body": "Fixture comment", "user": ACTIVE}])}},
            "IssueAttachments": {"issue": {"attachments": page([{"id": "attachment-1", "title": "Fixture link", "url": NOTE["url"]}])}},
            "TeamLabels": {"team": {"labels": page([{"id": "label-1", "name": "Regression"}])}},
            "TeamCycles": {"team": {"cycles": page([{"id": "cycle-1", "number": 5}])}},
            "TeamStates": {"team": {"states": page([ISSUE["state"]])}},
            "ProjectStatuses": {"projectStatuses": page([PROJECT["status"]])},
            "Users": {"users": page([ACTIVE])},
        }
        if name in bodies:
            return {"data": bodies[name]}
        if not query.startswith("mutation "):
            raise AssertionError(f"Unknown operation: {name}")
        field = re.search(r"\{\s*(\w+)\(", query).group(1)
        self.mutations.append((name, v))
        payload = {"success": True}
        if field.startswith("issue"):
            payload["issue"] = self.issues
        elif field in {"projectCreate", "projectUpdate"}:
            payload["project"] = PROJECT
        elif field in {"documentCreate", "documentUpdate"}:
            payload["document"] = DOCUMENT
        elif field == "commentCreate":
            payload["comment"] = {"id": "new-comment", "url": NOTE["url"]}
        elif field == "notificationUpdate":
            for note in self.notifications:
                if note["id"] == v["id"]:
                    note.update(v["input"])
        elif field == "notificationArchive":
            self.notifications = [note for note in self.notifications if note["id"] != v["id"]]
        elif field == "inboxNotificationUpdate":
            self.grouped = [note for note in self.grouped if note["id"] != v["id"]]
        return {"data": {field: payload}}


def server(fixture):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_POST(self):
            if self.path != "/graphql" or self.headers["Authorization"] != TEST_KEY:
                self.send_error(401)
                return
            request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            reply = json.dumps(fixture.request(request["query"], request["variables"])).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(reply)))
            self.end_headers()
            self.wfile.write(reply)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


class Host:
    def __init__(self, binary, env, root):
        self.process = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, env=env)
        self.frames = queue.Queue()
        self.calls = []
        threading.Thread(target=self.read, daemon=True).start()
        hello = self.frames.get(timeout=15)
        assert hello["type"] == "hello" and hello["id"] == MANIFEST["id"] and hello["protocol"] == 1
        (root / "data").mkdir()
        (root / "tmp").mkdir()
        result = self.send("initialize", {
            "protocol": 1, "extensionId": MANIFEST["id"], "platform": "windows",
            "packageDir": str(ROOT), "dataDir": str(root / "data"), "tempDir": str(root / "tmp"),
            "grantedCapabilities": MANIFEST["capabilities"], "reason": "fixture", "migrationPaths": {},
        })
        assert result["ready"], result

    def read(self):
        for line in self.process.stdout:
            assert TEST_KEY.encode() not in line, "Key leaked to protocol"
            self.frames.put(json.loads(line))
        self.frames.put(None)

    def write(self, obj):
        self.process.stdin.write((json.dumps(obj) + "\n").encode())
        self.process.stdin.flush()

    def send(self, method, params, error=False):
        request = f"h:{uuid.uuid4()}"
        self.write({"type": "request", "id": request, "method": method, "params": params})
        while True:
            frame = self.frames.get(timeout=40)
            assert frame is not None, self.process.stderr.read()
            if frame["type"] == "request":
                self.calls.append((frame["method"], frame["params"]))
                reply = {"response": 1} if frame["method"] == "host.dialog.confirm" else True
                self.write({"type": "response", "id": frame["id"], "result": reply})
                continue
            if frame["type"] == "event":
                continue
            assert frame["type"] == "response" and frame["id"] == request, frame
            if error:
                assert "error" in frame, frame
                return frame["error"]
            assert "error" not in frame, frame
            return frame["result"]

    def run(self, name, prefs=None, args=None):
        return self.send("session.open", {"sessionId": name, "commandName": name, "preferences": prefs or {},
                                         "argumentValues": args or {}})

    def op(self, name, operation, payload=None, error=False):
        return self.send("session.call", {"sessionId": name, "operation": "extension:" + operation,
                                          "payload": payload or {}}, error=error)

    def action(self, name, action, fields=None):
        return self.op(name, "invoke-action", {"actionId": action, "formValues": fields or {}})

    def stop(self):
        self.send("shutdown", {})
        self.process.wait(timeout=10)
        error = self.process.stderr.read()
        assert self.process.returncode == 0 and not error, error

    def abort(self):
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait(timeout=5)


def action(view, title, row=0):
    by_id = {item["id"]: item["title"] for item in view["actions"]}
    if row is not None and view["root"]["type"] == "List":
        ids = view["root"]["children"][row]["props"]["actionIds"]
    elif row is None:
        ids = view["root"]["props"]["actionIds"]
    else:
        ids = list(by_id)
    return next(action for action in ids if by_id[action] == title)


def first(reply):
    return reply["root"]["children"][0]["props"]["title"]


def active_menu(host):
    return next(params for method, params in reversed(host.calls) if method == "host.tray.set")


def exercise(host, fixture):
    prefs = {"linearApiKey": TEST_KEY}
    search = "search-issues"
    search_reply = host.run(search, dict(prefs, limit="1", issueCustomCopyAction="{ISSUE_ID}: {ISSUE_TITLE}"))
    assert first(search_reply) == "Fixture issue" and search_reply["root"]["props"]["pagination"]["hasMore"]
    assert fixture.requests[-1][1]["first"] == 1
    more = host.op(search, "load-more")
    assert len(more["root"]["children"]) == 2 and fixture.requests[-1][1]["after"] == "issues-page-2"
    results = host.op(search, "search-text-changed", {"searchText": "fixture"})
    assert fixture.requests[-1] == ("SearchIssues", {"after": None, "filter": {}, "first": 1, "term": "fixture"})
    fixture.invalid_cursors = True
    assert "cursor" in host.op(search, "load-more", error=True)["message"]
    fixture.invalid_cursors = False
    error = host.op(search, "search-text-changed", {"searchText": "echo error"}, error=True)
    assert TEST_KEY not in error["message"]
    copied = host.action(search, action(results, "Copy Custom Issue Format"))
    assert host.calls[-1] == ("host.clipboard.write", {"text": "SYN-1: Fixture issue"})
    detail = host.action(search, action(copied, "Show Details"))
    assert "Fixture issue body" in detail["root"]["props"]["markdown"]
    assigned = host.action(search, action(detail, "Assign to Me"))
    assert fixture.mutations[-1] == ("IssueUpdate", {"id": "issue-1", "input": {"assigneeId": "user-1"}})
    comments = host.action(search, action(assigned, "Show Comments"))
    assert first(comments) == "Fixture comment"
    comment_form = host.action(search, action(comments, "Add Comment", None))
    host.action(search, action(comment_form, "Add Comment", None), {"body": "New fixture comment"})
    assert fixture.mutations[-1][1]["input"] == {"issueId": "issue-1", "body": "New fixture comment"}

    assigned = host.run("assigned-issues", prefs)
    dropdown = assigned["root"]["props"]["searchBarAccessory"]["props"]["actionId"]
    host.action("assigned-issues", dropdown, {"value": "subscribed"})
    assert fixture.requests[-1][1]["filter"] == {"subscribers": {"some": {"isMe": {"eq": True}}}}
    assert first(host.run("created-issues", prefs)) == "Fixture issue"
    cycle = host.run("active-cycle", prefs)
    assert fixture.requests[-1][1]["id"] == "cycle-1"
    host.action("active-cycle", cycle["root"]["props"]["searchBarAccessory"]["props"]["actionId"], {"value": "team-2"})
    assert fixture.requests[-1][1]["id"] == "cycle-2"

    projects = host.run("search-projects", dict(prefs, primaryAction="openInLinear"))
    host.action("search-projects", action(projects, "Open in Linear"))
    assert host.calls[-1] == ("host.shell.open", {"target": PROJECT["url"]})
    projects = host.op("search-projects", "refresh-session")
    detail = host.action("search-projects", action(projects, "Show Details"))
    milestones = host.action("search-projects", action(detail, "Show Milestones"))
    assert first(milestones) == "First milestone"
    host.action("search-projects", action(milestones, "Show Milestone Issues"))
    assert fixture.requests[-1][1]["filter"]["projectMilestone"]["id"]["eq"] == "milestone-1"

    views = host.run("search-custom-views", prefs)
    assert len(views["root"]["children"]) == 1
    view_detail = host.action("search-custom-views", action(views, "Show Details"))
    assert first(host.action("search-custom-views", action(view_detail, "Show View Issues"))) == "Fixture issue"
    docs = host.run("search-documents", prefs)
    assert "Synthetic Document" in host.action("search-documents", action(docs, "Show Details"))["root"]["props"]["markdown"]
    assert first(host.run("favorites", prefs)) == "Fixture issue"

    create = host.run("create-issue", prefs)
    assert create["root"]["type"] == "Form"
    team_field = next(field for field in create["root"]["children"] if field["props"]["id"] == "teamId")
    create = host.action("create-issue", team_field["props"]["actionId"],
                         {"value": "team-2", "teamId": "team-2", "priority": "1", "title": "Urgent fixture"})
    priority_field = next(field for field in create["root"]["children"] if field["props"]["id"] == "priority")
    assert priority_field["props"]["value"] == "1", "Dependent field rerender must preserve string-valued priority"
    invalid = host.action("create-issue", action(create, "Create Issue", None), {"title": ""})
    assert invalid["notice"] == "Title is required."
    created = host.action("create-issue", action(invalid, "Create Issue", None),
                          {"title": "Synthetic bug", "teamId": "team-1", "priority": "1",
                           "labelIds": ["label-1"], "dueDate": "2026-12-15T00:00:00Z"})
    assert fixture.mutations[-1][1]["input"]["dueDate"] == "2026-12-15"
    host.action("create-issue", action(created, "Copy Issue ID", None))
    assert host.calls[-1] == ("host.clipboard.write", {"text": "SYN-1"})

    create = host.run("create-project", prefs)
    result = host.action("create-project", action(create, "Create Project", None),
                         {"name": "Fixture project", "teamIds": ["team-1"], "memberIds": ["user-1"]})
    assert "Project created" in result["root"]["props"]["markdown"]
    assert fixture.mutations[-1][1]["input"]["memberIds"] == ["user-1"]

    quick = host.run("create-issue-for-myself", dict(prefs, preferredTeamKey="SYN", preferredStatusName="Todo",
                                                   shouldCloseMainWindow=True), {"title": "Quick", "description": "Some notes"})
    assert quick["mode"] == "no-view" and fixture.mutations[-1][1]["input"]["stateId"] == "status-1"
    assert host.calls[-1] == ("host.window.visibility", {"visible": False})
    quick = host.run("quick-add-comment-to-issue", prefs, {"issueId": "SYN-1", "comment": "Quick comment"})
    assert quick["mode"] == "no-view" and fixture.mutations[-1][1]["input"]["body"] == "Quick comment"

    notifications = host.run("notifications", prefs)
    updated = host.action("notifications", action(notifications, "Open Notification"))
    assert host.calls[-1] == ("host.shell.open", {"target": "https://linear.app" + NOTE["url"]})
    assert fixture.mutations[-1][1]["id"] == NOTE["id"] and fixture.mutations[-1][1]["input"]["readAt"]
    updated = host.action("notifications", action(updated, "Mark as Unread"))
    assert fixture.mutations[-1][1]["input"]["readAt"] is None
    host.action("notifications", action(updated, "Archive Notification"))
    assert fixture.mutations[-1][0] == "NotificationArchive"

    unread = host.run("unread-notifications", dict(prefs, groupNotifications=True))
    assert unread["mode"] == "no-view"
    assert ("host.background.retain", {"key": "linear-inbox", "durable": False}) in host.calls
    menu = active_menu(host)
    assert "1 unread" in menu["tooltip"]
    assert any(item.get("commandName") == "notifications" for item in menu["items"])
    group = next(item for item in menu["items"] if item.get("kind") == "submenu")
    old_read_all = next(item["id"] for item in menu["items"] if item.get("title") == "Mark All as Read")
    host.send("background.event", {"resourceId": "linear-inbox", "actionId": group["items"][1]["id"]})
    assert fixture.mutations[-1] == ("ReadInboxGroup", {"id": "group-1"})
    assert "0 unread" in active_menu(host)["tooltip"]
    disable = next(item["id"] for item in active_menu(host)["items"] if item.get("title") == "Stop Inbox Menu")
    host.send("background.event", {"resourceId": "linear-inbox", "actionId": disable})
    assert host.calls[-2:] == [("host.tray.remove", {"resourceId": "linear-inbox"}),
                              ("host.background.release", {"key": "linear-inbox"})]
    host.send("session.dispose", {"sessionId": "unread-notifications"})
    fixture.grouped = [{"id": "group-2", "title": "Different notification after restart", "url": NOTE["url"]}]
    host.run("unread-notifications", dict(prefs, groupNotifications=True))
    mutation_count, host_call_count = len(fixture.mutations), len(host.calls)
    for old_action in [group["items"][1]["id"], old_read_all, disable]:
        error = host.send("background.event", {"resourceId": "linear-inbox", "actionId": old_action}, error=True)
        assert "menu changed" in error["message"]
    assert len(fixture.mutations) == mutation_count and len(host.calls) == host_call_count
    fixture.grouped = []
    host.send("session.dispose", {"sessionId": "unread-notifications"})
    host.run("unread-notifications", dict(prefs, alwaysShow=False))
    assert host.calls[-1] == ("host.tray.remove", {"resourceId": "linear-inbox"})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--production-smoke", action="store_true")
    args = parser.parse_args()
    binary = args.binary.resolve()
    protocol = json.loads(subprocess.check_output([binary, "--protocol-info"], timeout=15))
    assert protocol["id"] == MANIFEST["id"] and protocol["version"] == MANIFEST["version"]
    assert protocol["protocol"] == MANIFEST["protocolVersion"] and protocol["commands"] == COMMANDS
    assert subprocess.run([binary, "--self-test"], capture_output=True, timeout=15).returncode == 0
    fixture = Fixture()
    with server(fixture) as mock, tempfile.TemporaryDirectory() as tmp:
        env = dict(os.environ, TEZBAR_LINEAR_TEST_ENDPOINT=f"http://127.0.0.1:{mock.server_port}/graphql")
        host = Host(binary, env, Path(tmp))
        try:
            setup = host.run("search-custom-views")
            assert "Connect Linear" in setup["root"]["props"]["markdown"]
            host.action("search-custom-views", action(setup, "Open Extension Settings", None))
            assert host.calls[-1] == ("host.settings.open", {"section": "settings-extensions"})
            host.send("session.dispose", {"sessionId": "search-custom-views"})
            assert not fixture.requests
            if not args.production_smoke:
                exercise(host, fixture)
            host.stop()
        finally:
            host.abort()
            mock.shutdown()
    mode = "production smoke" if args.production_smoke else "HTTP fixture and JSONL"
    print(f"Passed {mode}, all 14 command IDs, {len(fixture.requests)} local GraphQL requests, "
          f"{len(fixture.mutations)} fixture mutations.")


if __name__ == "__main__":
    main()
