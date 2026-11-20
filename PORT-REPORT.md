# Linear Rust port

The standalone port implements the 14 Linear Store command IDs for the macOS and
Windows manifest. It uses the published Tezbar Rust extension SDK 0.1.1 at
`1a11afc14693cc6463e0b95ae7d61991b4a60427` and a pinned Cargo.lock. It
runs as a schema 3 protocol 1 executable.

## Provenance

- Raycast Store rank 7 in the pinned inventory, 421,138 downloads at capture.
- Source commit `7ad5b7f3b32d2168eab8c7d4ea6001cede9a2ba8` under
  `extensions/linear`. All 486 upstream files and the repository MIT license
  match GitHub's Git blob IDs. SOURCE.json records each blob ID and SHA-256.
- The native manifest preserves the Store command IDs, titles, descriptions,
  modes, arguments, preferences, and macOS/Windows platforms. It adds the
  `linearApiKey` password preference for explicit personal key setup. The
  archive includes the original resource files, Store manifest, AI configuration
  and skills, source hash inventory, and license notices.
- [Pinned Raycast extension](https://github.com/raycast/extensions/tree/7ad5b7f3b32d2168eab8c7d4ea6001cede9a2ba8/extensions/linear)
- [Linear GraphQL authentication and operation reference](https://linear.app/developers/graphql)
- [Linear cursor pagination](https://linear.app/developers/pagination)

## Command coverage

| Command | Rust behavior |
| --- | --- |
| `create-issue` | Team-scoped form for title, Markdown description, status, assignee, priority, labels, cycle, project, milestone, estimate and due date. Issue create result supports opening the issue and the selected copy action. |
| `search-issues` | Recent issues and full-text search with cursor paging, issue details, edit, assignment, subscription, archive, parent/sub-issues, comments, link attachments and copy actions. |
| `assigned-issues` | Assigned, created and subscribed list filters with cursor paging and shared issue actions. |
| `created-issues` | Paginated issues created by the current user and shared issue actions. |
| `active-cycle` | Team switching and paginated issues in the chosen team's active cycle. Create Issue uses that team and cycle. |
| `search-projects` | Workspace or team search with cursor paging. Project details expose paginated issues, status updates, milestones and documents, with project edit/archive and forms to create associated milestones and documents. |
| `search-custom-views` | Paginated issue custom views with nested issue lists and original Linear links. |
| `search-documents` | Paginated document title search, Markdown detail, browser opening, copy link and content edit. |
| `create-project` | Form for teams, name, description, lead, members, status, start and target dates. |
| `notifications` | Paginated inbox, unread and snoozed filters, open and mark read, mark unread, snooze one hour, archive, and issue details. |
| `unread-notifications` | Retained host tray item, 15-minute polling, optional grouping, empty-state visibility, open/read, mark all read, and launch notifications. Stop Inbox Menu releases tray ownership. |
| `create-issue-for-myself` | Argument-based issue creation with workspace viewer assignment, preferred team and status, and optional close window. |
| `quick-add-comment-to-issue` | Resolve human issue identifier, add comment and optionally close window. |
| `favorites` | Cursor-paged Linear favorites with links, native entity details, and custom view traversal. |

## Remaining differences and acceptance

- The port uses an explicit personal API key in the extension password
  preference. Raycast's managed read/write OAuth, its token refresh and
  account migration do not carry over. A key can have different team,
  endpoint and write permissions. In particular, Linear's grouped inbox
  fields are used with the same raw operations as upstream and still need
  confirmation with a real personal API key.
- Native views cover the core workflows. Raycast form drafts, issue template
  application, interactive mention and typeahead pickers, bundled local-file
  attachment upload, project/initiative document move or delete, and Raycast's
  document creator variants remain gaps. The Rust attachment form handles links.
  Shell opening hands HTTPS links to the system handler. The port does not
  select an installed Linear app or replicate Raycast deep-link integration.
- Scope: 14 command entry points. The original AI tools, AI skill configuration
  and resource provider are retained for provenance. The SDK has no
  equivalent dispatch for these Raycast features.
- The retained tray stops at process or app exit. Restart it with Unread
  Notifications. Actual Windows tray rendering and GPUI form/desktop behavior
  require a desktop run. Apple Silicon build, packaging/signing and desktop
  acceptance remain pending. A real Linear account and API key were never used.
- Lists page through results and use bounded in-process caches. Issue pages
  have at most 50 results, and label choices use the configured API label
  limit up to 350. The tray displays the first 35 unread entries while its
  count and bulk read operation use the fetched set, capped at 400 entries.

## Verification

- Local x86_64-pc-windows-msvc with rustc 1.98.1 and `CARGO_BUILD_JOBS=1`.
- `cargo test --locked` passed two unit tests covering date conversion and
  allowed web URL schemes.
- The real fixture executable's `--protocol-info` and `--self-test` passed.
  The fake JSONL host exercised every command, 51 loopback-only GraphQL requests,
  and ten synthetic mutations. Checks include the exact auth header, pagination
  cursors, repeated cursor rejection, GraphQL error redaction, scoped
  clipboard/settings/shell/window calls, form validation, tray retention,
  grouped read mutation, and tray stop and empty state.

- The production release executable passed `--protocol-info`, `--self-test`,
  and the real process JSONL configuration smoke. The production smoke made
  zero HTTP requests. The production executable statically excludes the
  fixture endpoint hook.
- The Windows x64 development ZIP contains 579 files
  including its manifest. All 578 inventoried
  entries passed SHA-256, CRC, duplicate/case-collision, Windows path and
  original Store manifest parity checks. The archive is 6,751,143 bytes.
- Archive SHA-256: `4d1d48375d610338d0d103839e8dba512ed75712ac5945eda8b56de58d08bee2`
- Executable SHA-256: `43de382c670dfe296e0381e8f1164a2ab56506a5f9a5c8f84a32481da336efcf`
- Archive path: `C:/Users/alikh/Tezbar-rust-ports/linear/dist/linear-x86_64-pc-windows-msvc.zip`
