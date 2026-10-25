# VS Code agent host conformance report

Results of running the [AHP conformance suite](../README.md) against the real VS Code agent host,
triaged against the **AHP 1.0.0** specification.

| | |
| --- | --- |
| Host | Visual Studio Code Agent Host **1.144.0 Insiders** (server commit `7cc5e7382b28`), started standalone with `code-insiders agent host --new-instance` |
| Agent / model | `copilotcli` (Copilot) / `gpt-6-luna` (GPT-6 Luna), authenticated with a GitHub token |
| Protocol | Host natively speaks `1.1.0`, advertises `1.0.0`, `0.10.0`, `0.9.0`; the suite negotiates **`1.0.0`** |
| Config | [`examples/vscode-agent-host.yml`](../examples/vscode-agent-host.yml) |
| Date | 2026-10-24 |
| Result | **330 passed, 38 failed, 92 skipped** of 460, run with `--proposed`. Failures were stable across full runs except where a test needs optional output the host didn't produce (noted per finding) |

Every failing test was judged against the 1.0.0 text (proposed tests against the pinned protocol
`main`). Where the suite asserted something the spec doesn't require, the test was relaxed
([test changes](#suite-changes-made-for-this-host)); the remaining 38 failures are **26 genuine
deviations**, below. Severity: **High** breaks interoperability for a
client that follows the spec; **Medium** violates a MUST with limited impact; **Low** violates a SHOULD.

## Summary

| # | Severity | Finding | Failing tests |
| --- | --- | --- | --- |
| 1 | High | [Sessions only initialize after their first turn](#1-sessions-only-initialize-after-their-first-turn) | `sessions/create-session/ready` |
| 2 | High | [Idle steering messages start a turn (1.1 semantics on a 1.0 connection)](#2-idle-steering-messages-start-a-turn-11-semantics-on-a-10-connection) | `chats/pending-message-set/idle-steering-retained`, `chats/pending-message-set/mirror-matches-snapshot` |
| 3 | High | [Fork creation accepts unknown turns, cross-session sources, and active turns](#3-fork-creation-accepts-unknown-turns-cross-session-sources-and-active-turns) | `chats/fork/{unknown-turn,cross-session-source,active-turn}-rejected` |
| 4 | High | [Advertised working-directory mutation is refused to non-editor clients](#4-advertised-working-directory-mutation-is-refused-to-non-editor-clients) | `tools/working-directories/add-directory` |
| 5 | Medium | [Malformed JSON-RPC requests are ignored or executed](#5-malformed-json-rpc-requests-are-ignored-or-executed) | `transport/json-rpc/{missing-method,non-string-method,missing-jsonrpc,wrong-jsonrpc-version,non-object-request}` |
| 6 | Medium | [Connection stays open after `UnsupportedProtocolVersion`](#6-connection-stays-open-after-unsupportedprotocolversion) | `transport/initialize/unsupported-version-closes-connection` |
| 7 | Medium | [Duplicate session URI returns `-32002` instead of `-32003`](#7-duplicate-session-uri-returns--32002-instead-of--32003) | `sessions/create-session/duplicate-uri` |
| 8 | Medium | [Cancelling an idle chat is not rejected](#8-cancelling-an-idle-chat-is-not-rejected) | `chats/turn-cancelled/idle-rejected` |
| 9 | Medium | [Confirming a completed tool call is not rejected](#9-confirming-a-completed-tool-call-is-not-rejected) | `tools/confirmations/reject-completed-tool-confirmation` |
| 10 | Medium | [`resourceResolve` returns the link URI when following symlinks](#10-resourceresolve-returns-the-link-uri-when-following-symlinks) | `resources/resource-resolve/follow-symlink-default` |
| 11 | Medium | [`createResourceWatch` reuses channel URIs](#11-createresourcewatch-reuses-channel-uris) | `resources/create-resource-watch/fresh-channel` |
| 12 | Low | [`listSessions` is ordered oldest first](#12-listsessions-is-ordered-oldest-first) | `sessions/list-sessions/modified-first` |
| 13 | High | [Session disposal omits `session/chatRemoved`](#13-session-disposal-omits-sessionchatremoved) | `sessions/dispose-session/emits-chat-removed` |
| 14 | High | [Invalid chat attachments are accepted](#14-invalid-chat-attachments-are-accepted) | `chats/attachments/{unknown-chat,unknown-end-turn,active-end-turn,non-retained-end-turn}-rejected` |
| 15 | High | [Marking a chat read also marks its session read](#15-marking-a-chat-read-also-marks-its-session-read) | `chats/is-read-changed/default-chat` |
| 16 | High | [Binary `resourceRead` with a UTF-8 preference corrupts bytes](#16-binary-resourceread-with-a-utf-8-preference-corrupts-bytes) | `resources/resource-read/binary-utf8-fallback` |
| 17 | Medium | [Root notifications reach session-only subscribers](#17-root-notifications-reach-session-only-subscribers) | `sessions/root-notifications/{removed,summary}-requires-subscription` |
| 18 | Medium | [An invalid token is acknowledged](#18-an-invalid-token-is-acknowledged) | `auth/authenticate/invalid-advertised-token-rejected` |
| 19 | Medium | [Duplicate automation disable conditions are persisted](#19-duplicate-automation-disable-conditions-are-persisted) | `automations/validation/duplicate-disable-kind-rejected` |
| 20 | Medium | [Terminal `cwd` is a filesystem path, not a URI](#20-terminal-cwd-is-a-filesystem-path-not-a-uri) | `terminals/create-terminal/create-initial-state` |
| 21 | Low | [Input actions for unknown requests are accepted](#21-input-actions-for-unknown-requests-are-accepted) | `chats/input/unknown-{answer,completion}-request-rejected` |
| 22 | Low | [Queued-message consumption skips `pendingMessageRemoved`](#22-queued-message-consumption-skips-pendingmessageremoved) | `chats/queued/consume-head-after-active-turn` |
| 23 | Low | [Non-owners can complete or update client tools](#23-non-owners-can-complete-or-update-client-tools) | `tools/client-tools/non-owner-{completion,content}-rejected` |
| 24 | Low | [Unsubscribing doesn't remove an active client](#24-unsubscribing-doesnt-remove-an-active-client) | `sessions/active-clients/unsubscribe-removes-membership` |
| 25 | Low | [A non-holder can take over a terminal claim](#25-a-non-holder-can-take-over-a-terminal-claim) | `terminals/claimed/nonholder-claim-rejected` |
| 26 | Low | [Mixed-case log levels change the export channel](#26-mixed-case-log-levels-change-the-export-channel) | `telemetry/logs/level-case-insensitive-subscription` |

Results by area:

| Area | Passed | Failed | Skipped |
| --- | --- | --- | --- |
| Transport & lifecycle | 69 | 6 | 4 |
| Root & sessions | 75 | 7 | 13 |
| Chats & turns | 61 | 14 | 19 |
| Resources | 68 | 3 | 2 |
| Tools & permissions | 22 | 4 | 13 |
| Authentication | 10 | 1 | 18 |
| Terminals | 14 | 2 | 1 |
| Automations | 6 | 1 | 2 |
| Telemetry | 1 | 0 | 3 |
| Proposed | 4 | 0 | 17 |

To reproduce any finding, from `tools/ahp-conformance`:

```sh
export AHP_VSCODE_TOKEN="$(openssl rand -hex 24)"
node src/cli.ts examples/vscode-agent-host.yml --filter '<test id>'
```

Wire excerpts below are abbreviated and redacted.

## Findings

### 1. Sessions only initialize after their first turn

**Severity:** High. **Test:** `sessions/create-session/ready`.

**Spec:** `docs/specification/session-channel.md#lifecycle`: "4. Server creates session with
lifecycle: 'creating', returns the snapshot. 5. Server asynchronously initialises the agent backend.
6. On success: server dispatches session/ready … 8. Server broadcasts root/sessionAdded".
`#active-session`: "Once a session reaches `lifecycle: 'ready'`, clients may create chats on it".

**Observed:** After `createSession` the session stays `creating` indefinitely (no `session/ready` or
`session/creationFailed` within 90 s), is absent from `listSessions`, and no `root/sessionAdded` is
broadcast. The first `chat/turnStarted` on the default chat triggers initialization; `session/ready`
follows ~100 ms later, then the catalog entry.

```jsonc
→ {"id":4,"method":"createSession","params":{"channel":"ahp-session:/S","provider":"copilotcli","workingDirectories":["file:///W"]}}
← {"id":4,"result":null}
← {"id":5,"result":{"snapshot":{"resource":"ahp-session:/S","state":{"lifecycle":"creating","defaultChat":"ahp-chat:/C",…}}}}
   // no further session lifecycle actions until a turn is started
```

**Expected:** Initialization starts from `createSession` and resolves to `session/ready` or
`session/creationFailed` without a turn. A client that waits for `ready` before chatting or calling
`createChat` (as the lifecycle implies) deadlocks against this host.

**Suite impact:** handled by the `quirks.lazySessionInit` config flag (see below), so this one
deviation fails only this test. Catalog, notification, and chat-creation tests pass once the
session has been initialized.

### 2. Idle steering messages start a turn (1.1 semantics on a 1.0 connection)

**Severity:** High. **Tests:** `chats/pending-message-set/idle-steering-retained`,
`chats/pending-message-set/mirror-matches-snapshot`.

**Spec:** `docs/specification/versioning.md`: "On a successful negotiation the host MUST NOT use
newer-version-only behaviors on that connection unless gated behind a capability the client has
acknowledged." `docs/specification/chat-channel.md#steering-messages`: "Steering messages added while
idle are silently stored and consumed when a turn becomes active." In 1.0.0 a steering
`chat/pendingMessageSet` "always replaces the single steering message" (`ChatState.steeringMessage`).

**Observed:** On an idle chat, a steering message is immediately promoted to a new turn instead of
being stored; a following steering update is then rejected as "being processed". Steering entries
also carry the 1.1-only `kind` field (harmless on its own; see [observations](#other-observations)).

```jsonc
→ dispatchAction chat/pendingMessageSet {"id":"first","kind":"steering","message":{"text":"first",…}}
← action chat/turnStarted {"turnId":"…","queuedMessageId":"first","message":{"text":"first",…}}
→ dispatchAction chat/pendingMessageSet {"id":"second","kind":"steering","message":{"text":"updated",…}}
← action chat/pendingMessageSet … "rejectionReason":"Cannot change a steering message while it is being processed."
```

**Expected:** With 1.0.0 negotiated, keep idle steering in the `steeringMessage` slot (replacing on
each set) until an active turn consumes it; never start model work from a steering set.

### 3. Fork creation accepts unknown turns, cross-session sources, and active turns

**Severity:** High. **Tests:** `chats/fork/unknown-turn-rejected`,
`chats/fork/cross-session-source-rejected`, `chats/fork/active-turn-rejected`.

**Spec:** `docs/specification/chat-channel.md#creation`: "It MUST also reject a source chat outside
the target session, an unknown source chat or turn, or a source that names the chat being created."
and "For forks, the host MUST additionally reject any source whose `kind` is not `"fork"` — forks only
target completed turns."

**Observed:** All three requests succeed on a ready session (the third forks from a still-running turn).

```jsonc
→ {"id":7,"method":"createChat","params":{"channel":"ahp-session:/T","chat":"ahp-chat:/F","source":{"kind":"fork","chat":"ahp-chat:/T-default","turnId":"missing"}}}
← {"id":7,"result":null}
→ {"id":10,"method":"createChat","params":{"channel":"ahp-session:/T2","chat":"ahp-chat:/F2","source":{"kind":"fork","chat":"ahp-chat:/OTHER-SESSION-default","turnId":"<completed turn>"}}}
← {"id":10,"result":null}
```

**Expected:** A JSON-RPC error for each (no specific code is mandated).

### 4. Advertised working-directory mutation is refused to non-editor clients

**Severity:** High. **Test:** `tools/working-directories/add-directory`.

**Spec:** `AgentCapabilities` (`types/channels-root/state.ts`): presence of a capability "signals
support". `MultipleWorkingDirectoriesCapability.immutablePrimary`: "Additional directories after it
remain equal peers that can be added and removed freely." `session/workingDirectorySet` is
client-dispatchable and "Only valid when the agent advertises
`AgentCapabilities.multipleWorkingDirectories`".

**Observed:** The agent advertises `multipleWorkingDirectories: { immutablePrimary: true }`, but adding
a secondary directory to an initialized session is rejected based on client identity.

```jsonc
→ dispatchAction session/workingDirectorySet {"directory":"file:///W/secondary"}
← action session/workingDirectorySet … "rejectionReason":"Session working-directory actions require an Editor Window client."
```

**Expected:** Accept the addition, or don't advertise the capability to clients that may not use it.
1.0.0 has no notion of an "Editor Window client".

### 5. Malformed JSON-RPC requests are ignored or executed

**Severity:** Medium. **Tests:** `transport/json-rpc/missing-method`, `non-string-method`,
`missing-jsonrpc`, `wrong-jsonrpc-version`, `non-object-request`.

**Spec:** `docs/specification/overview.md`: "AHP uses JSON-RPC 2.0 as its message framing." JSON-RPC
2.0 §4: `jsonrpc` "MUST be exactly "2.0""; `method` is "A String"; §5: "the Server MUST reply with a
Response, except for in the case of Notifications"; `-32600` "Invalid Request: The JSON sent is not
a valid Request object" (§7 shows a numeric `method` yielding `-32600`).

**Observed** (each on its own initialized connection):

```jsonc
→ {"jsonrpc":"2.0","id":71}                                   ← (no response)
→ {"jsonrpc":"2.0","id":72,"method":42,"params":{…}}          ← {"id":72,"error":{"code":-32601,"message":"Method not found: 42"}}
→ {"id":73,"method":"ping","params":{…}}                      ← {"id":73,"result":null}
→ {"jsonrpc":"1.0","id":74,"method":"ping","params":{…}}      ← {"id":74,"result":null}
→ 42                                                          ← (no response)
```

**Expected:** `-32600 Invalid Request` for each (with `id: null` when no id can be read).

### 6. Connection stays open after `UnsupportedProtocolVersion`

**Severity:** Medium. **Test:** `transport/initialize/unsupported-version-closes-connection`.

**Spec:** `docs/specification/versioning.md#negotiation`, step 3: "it MUST respond with
`UnsupportedProtocolVersion` (`-32005`) and required `data.supportedVersions` instead of a result,
and close the connection."

**Observed:** The error and `supportedVersions` are correct, but the socket stays open and still
answers `ping` 5 s later.

```jsonc
→ {"id":1,"method":"initialize","params":{…,"protocolVersions":[]}}
← {"id":1,"error":{"code":-32005,…,"data":{"supportedVersions":["1.0.0","0.10.0","0.9.0"],…}}}
→ {"id":2,"method":"ping","params":{"channel":"ahp-root://"}}   // 5 s later
← {"id":2,"result":null}
```

**Expected:** Close the transport after sending the error.

### 7. Duplicate session URI returns `-32002` instead of `-32003`

**Severity:** Medium. **Test:** `sessions/create-session/duplicate-uri`.

**Spec:** `createSession`: "If the session URI already exists, the server MUST return an error with
code `-32003` (`SessionAlreadyExists`)." `-32002` is `ProviderNotFound`.

**Observed:**

```jsonc
→ {"id":4,"method":"createSession","params":{"channel":"ahp-session:/S","provider":"copilotcli",…}}   ← {"id":4,"result":null}
→ {"id":7,"method":"createSession","params":{"channel":"ahp-session:/S","provider":"copilotcli"}}
← {"id":7,"error":{"code":-32002,"message":"Conflicting chat storage identity: ahp-chat:/C"}}
```

**Expected:** `-32003`, so clients can tell an identity collision from a missing provider.

### 8. Cancelling an idle chat is not rejected

**Severity:** Medium. **Test:** `chats/turn-cancelled/idle-rejected`.

**Spec:** `docs/specification/chat-channel.md#server-validation-of-client-actions`: "Invalid actions
MUST be echoed back with a `rejectionReason`"; for `chat/turnCancelled` with "No active turn":
"Server MUST reject the action".

**Observed:** On a chat with no turns, the cancellation is echoed as accepted (no `rejectionReason`).

```jsonc
→ dispatchAction chat/turnCancelled {"turnId":"missing","duration":0}
← action chat/turnCancelled {"turnId":"missing","duration":0}   // origin set, no rejectionReason
```

### 9. Confirming a completed tool call is not rejected

**Severity:** Medium. **Test:** `tools/confirmations/reject-completed-tool-confirmation`.

**Spec:** Same validation section: `chat/toolCallConfirmed` when the "Tool call [is] not in
`pending-confirmation` state": "Server MUST reject the action".

**Observed:** After `chat/toolCallComplete` and `chat/turnComplete`, a `chat/toolCallConfirmed` for that
tool call is echoed without a `rejectionReason`.

### 10. `resourceResolve` returns the link URI when following symlinks

**Severity:** Medium. **Test:** `resources/resource-resolve/follow-symlink-default`.

**Spec:** `ResourceResolveParams.followSymlinks`: "When `true` (default), follow symlinks and report
the metadata of the link target — and set `uri` in the result to the canonical (realpath) URI."

**Observed:** Target metadata, but the link's URI (not a `/var` vs `/private/var` spelling issue).

```jsonc
→ {"method":"resourceResolve","params":{"channel":"ahp-root://","uri":"file:///W/link.txt"}}   // link.txt -> target.txt
← {"result":{"uri":"file:///W/link.txt","type":"file","size":14}}
```

**Expected:** `"uri":"file:///W/target.txt"`.

### 11. `createResourceWatch` reuses channel URIs

**Severity:** Medium. **Test:** `resources/create-resource-watch/fresh-channel`.

**Spec:** `docs/specification/resource-watch-channel.md#uri`: "The receiver allocates a fresh watch
channel URI for every successful `createResourceWatch` call".

**Observed:** Two identical calls (the first already subscribed) return the same deterministic URI
(`ahp-resource-watch://r/<base64 of the descriptor>`), so the two watches cannot have independent
subscription lifetimes.

**Expected:** A distinct opaque channel per call; the host may still share the underlying watcher.

### 12. `listSessions` is ordered oldest first

**Severity:** Low (SHOULD). **Test:** `sessions/list-sessions/modified-first`.

**Spec:** `docs/specification/root-channel.md#paginating-the-catalogue`: "The server SHOULD return
most-recently-modified entries first, so the first page is the immediately useful one."

**Observed:** With two freshly initialized sessions, the older one (`modifiedAt …11:10.704Z`) is listed
before the newer one (`…11:17.807Z`), on every run.

### 13. Session disposal omits `session/chatRemoved`

**Severity:** High. **Test:** `sessions/dispose-session/emits-chat-removed`.

**Spec:** `docs/specification/chat-channel.md#disposal`: "When a chat is removed (whether explicitly or
because its session was torn down), the server MUST update the session's `chats` catalog via
`session/chatRemoved` so subscribers can release their per-chat subscriptions."

**Observed:** A second client subscribed to the session sees `root/sessionRemoved` and the
`disposeSession` result, but no `session/chatRemoved` for the session's chat within 5 s.

### 14. Invalid chat attachments are accepted

**Severity:** High. **Tests:** `chats/attachments/{unknown-chat,unknown-end-turn,active-end-turn,non-retained-end-turn}-rejected`.

**Spec:** `docs/specification/chat-channel.md#pulling-a-chat-into-another-chat`: "When provided, `endTurn`
MUST reference a completed, retained turn. The host MUST reject an attachment that references an
unknown chat, specifies an unknown, active, or non-retained `endTurn`."

**Observed:** A `chat/turnStarted` whose message carries each kind of invalid `MessageChatAttachment` is
accepted and starts a turn.

### 15. Marking a chat read also marks its session read

**Severity:** High. **Test:** `chats/is-read-changed/default-chat`.

**Spec:** `docs/specification/chat-channel.md#state`: "The action toggles `SessionStatus.IsRead` only on
the addressed `ChatState`; it does not change the read state of the owning session or sibling chats …
neither action implies the other."

**Observed:**

```jsonc
→ dispatchAction {"channel":"ahp-chat:/C","action":{"type":"chat/isReadChanged","isRead":true}}
← action {"channel":"ahp-chat:/C",…,"action":{"type":"chat/isReadChanged","isRead":true}}
← action {"channel":"ahp-session:/S",…,"action":{"type":"session/isReadChanged","isRead":true}}   // host-originated
```

### 16. Binary `resourceRead` with a UTF-8 preference corrupts bytes

**Severity:** High. **Test:** `resources/resource-read/binary-utf8-fallback`.

**Spec:** `ResourceReadParams` (`types/common/commands.ts`): "Binary content (images, etc.) MUST use
`base64` encoding." `ResourceReadResult`: if the server can't provide the requested encoding, "it MUST
fall back to either `base64` or `utf-8`".

**Observed:** Reading the bytes `00 ff 80 c3 28 fe 0d 0a` with `encoding: "utf-8"` returns replacement
characters labelled `"encoding":"utf-8"`, `"contentType":"application/octet-stream"`; the original
bytes are unrecoverable.

### 17. Root notifications reach session-only subscribers

**Severity:** Medium. **Tests:** `sessions/root-notifications/removed-requires-subscription`,
`sessions/root-notifications/summary-requires-subscription`.

**Spec:** `docs/specification/subscriptions.md#protocol-notifications`: "Protocol notifications go only to
clients subscribed to the channel they target." `root/*` notifications target `ahp-root://`.

**Observed:** A client initialized with only a session subscription (no root subscription) receives
`root/sessionRemoved` and `root/sessionSummaryChanged` for that session.

### 18. An invalid token is acknowledged

**Severity:** Medium. **Test:** `auth/authenticate/invalid-advertised-token-rejected`.

**Spec:** `docs/specification/authentication.md#token-delivery`: "If the token is invalid or the resource is
unrecognized, the server MUST return a JSON-RPC error (e.g. `AuthRequired` `-32007` or `InvalidParams`
`-32602`)."

**Observed:** `authenticate` for the advertised `https://api.github.com` with a syntactically plausible but
invalid token returns `{}`. (This doesn't show an authentication bypass; it misreports the outcome.)

### 19. Duplicate automation disable conditions are persisted

**Severity:** Medium. **Test:** `automations/validation/duplicate-disable-kind-rejected`.

**Spec:** `docs/guide/automations.md#disable-conditions`: "Each kind may appear at most once; hosts MUST
reject create and update requests with duplicate kinds, even if their values are identical."

**Observed:** A disabled, manual-only `automation/createRequested` with two identical `afterRuns`
conditions is accepted and the authoritative entry keeps both.

### 20. Terminal `cwd` is a filesystem path, not a URI

**Severity:** Medium. **Test:** `terminals/create-terminal/create-initial-state`.

**Spec:** `TerminalState.cwd?: URI` (`docs/specification/terminal-channel.md#state`).

**Observed:** `createTerminal` with `"cwd":"file:///W/test"` yields a snapshot with `"cwd":"/W/test"`.

### 21. Input actions for unknown requests are accepted

**Severity:** Low (SHOULD). **Tests:** `chats/input/unknown-answer-request-rejected`,
`chats/input/unknown-completion-request-rejected`.

**Spec:** `docs/specification/chat-channel.md#server-validation-of-client-actions`: for
`chat/inputAnswerChanged` / `chat/inputCompleted` with "No input request with matching `requestId`":
"Server SHOULD reject the action".

**Observed:** Both actions with a never-issued `requestId` are echoed without a `rejectionReason`.

### 22. Queued-message consumption skips `pendingMessageRemoved`

**Severity:** Low (SHOULD). **Test:** `chats/queued/consume-head-after-active-turn`.

**Spec:** `docs/specification/chat-channel.md#queued-messages`: "When a turn completes and `queuedMessages`
is non-empty, the server SHOULD: 1. Dispatch `chat/pendingMessageRemoved` with `kind: 'queued'` for the
first queued message. 2. Dispatch `chat/turnStarted` … with `queuedMessageId`".

**Observed:** The queued head starts its turn without the preceding `chat/pendingMessageRemoved`.

### 23. Non-owners can complete or update client tools

**Severity:** Low (SHOULD). **Tests:** `tools/client-tools/non-owner-completion-rejected`,
`tools/client-tools/non-owner-content-rejected`.

**Spec:** `ChatToolCallCompleteAction` / `ChatToolCallContentChangedAction`: "The server SHOULD reject this
action if the dispatching client does not match the contributor's `clientId`."

**Observed:** A second client's `chat/toolCallComplete` and `chat/toolCallContentChanged` for another
client's running tool are accepted and applied.

### 24. Unsubscribing doesn't remove an active client

**Severity:** Low (SHOULD). **Test:** `sessions/active-clients/unsubscribe-removes-membership`.

**Spec:** `SessionActiveClientRemovedAction`: "The host SHOULD dispatch this automatically when a client
stops participating in the session — for example when it unsubscribes from the session channel".

**Observed:** After an accepted `session/activeClientSet`, the client unsubscribes (staying connected);
other subscribers see no `session/activeClientRemoved` within 5 s.

### 25. A non-holder can take over a terminal claim

**Severity:** Low (SHOULD). **Test:** `terminals/claimed/nonholder-claim-rejected`.

**Spec:** `TerminalClaimedAction`: "The server SHOULD reject if the dispatching client does not currently
hold the claim."

**Observed:** A second client's `terminal/claimed` for a terminal claimed by the first client is accepted.

### 26. Mixed-case log levels change the export channel

**Severity:** Low (SHOULD). **Test:** `telemetry/logs/level-case-insensitive-subscription`.

**Spec:** `docs/specification/telemetry-channel.md#filtering`: level short names are "case-insensitive", and
"Each distinct expansion is its own subscription URI from the server's point of view".

**Observed:** Subscribing to `ahp-otlp://logs/WaRn` succeeds, but exports arrive on
`"channel":"ahp-otlp://logs/warn"`, a channel the client never subscribed to. The test skips when no log
batch arrives in its 400 ms window, so this showed up in one of the two final runs.

## Configuration needed for this host

[`examples/vscode-agent-host.yml`](../examples/vscode-agent-host.yml) launches an isolated host
(`--new-instance --foreground` with its own user/server data in the run's scratch directory, so it
never touches an editor's agent host) and stops it afterwards. It relies on these suite options:

- **Connection token:** the host requires `?tkn=<token>`; supplied with `target.query.tkn` from
  `AHP_VSCODE_TOKEN`, which the launch command also writes to the host's `--connection-token-file`.
- **Authentication:** `copilotcli` advertises `https://api.github.com` as `required`; both advertised
  resources get `gh auth token` via `auth.tokens`.
- **`agent.catalogTimeoutMs`** (new): model catalogs are empty in the `initialize` snapshot and are
  published via `root/agentsChanged` only after authentication, so profile discovery now waits for the
  configured model.
- **`quirks.lazySessionInit`** (new): lets shared setup proceed while a session is still `creating`
  (finding 1). Tests that need an initialized session call `SessionHandle.ready()`, which under this
  quirk runs one short model turn first. The tests that assert initialization itself still fail.

## Suite changes made for this host

Tests were relaxed only where they asserted something AHP 1.0.0 does not require:

| Test(s) | Change | 1.0.0 rationale |
| --- | --- | --- |
| `transport/initialize/{skip-incompatible-versions,incompatible-offer}` | Derive incompatible versions from the host's advertised `supportedVersions` instead of assuming `0.10.0` is unsupported. | Hosts select "within a supported caret-compatible range"; "Development versions are independent of these advertised baselines." |
| `transport/initialize/{unsupported-version-data,unsupported-version-closes-connection}` | Trigger rejection with an empty offer. | No host can select from an empty offer; the error/close requirements are unchanged. |
| `transport/json-rpc/{missing-required-parameter,wrong-parameter-type,missing-client-id}`, `sessions/subscribe/invalid-channel-type` | Require a correlated result or error, not specifically `-32602`. | JSON-RPC: absent expected names "MAY result in an error"; 1.0.0 mandates no runtime schema validation, only "exactly one response". |
| `transport/reconnect/protocol-notifications-not-replayed`, `sessions/list-sessions/*`, `sessions/root-notifications/*`, `sessions/dispose-session/session-removed`, `chats/fork/*-rejected` | Await `session.ready()` before testing catalog, notification, or `createChat` behavior. | `root/sessionAdded` follows initialization; chats may be created once `ready`. Isolates finding 1. |
| `sessions/list-sessions/modified-first` | Create and initialize its own two sessions instead of relying on other tests' sessions. | Determinism only. |
| `auth/create-session/required-resource-auth-required` | Skip when an unauthenticated `createSession` is accepted (auth may be enforced when the agent is used); still checks the `AuthRequired` shape when rejected. | Returning `AuthRequired` is a SHOULD and may happen at "any command". |
| `chats/fork/copies-visible-history` | Ignore copied turns' top-level `id`. | A fork "copies source history … into the new chat's visible `turns`"; nothing requires preserving turn ids across chats. |
| `chats/fork/mirror-matches-snapshot`, `chats/pending-message-set/mirror-matches-snapshot` | Project the chat title from the session catalog (`session/chatUpdated`); ignore additive pending-entry fields. | 1.0.0 has no chat-channel title action; additive fields "MUST be ignored by older peers". |
| `chats/pending-message-set/steering-replaces-slot` | Compare a steering entry's `id` and `message` only. | Same additive-field rule; finding 2 covers the behavior. |
| `resources/create-resource-watch/*` | Establish event delivery (repeated changes to pre-existing sentinel files) before the subject change; don't require `added` for sentinels. | 1.0.0 has no watcher-armed signal or latency guarantee, and events "are coalesced into batches". Exact `added`/`updated`/`deleted` checks for the subject are kept. |
| `tools/working-directories/{add-existing-directory-no-op,mirror-matches-snapshot}` | Initialize the session first; test only no-duplication and mirror consistency (accepting or rejecting the action). | Acceptance is covered by `add-directory` (finding 4). |
| `resources/content-ref/*` | Use a smaller model stimulus and observe content references during the turn; skip if the host publishes none. | Publishing content references is optional; reading a published one is required. |
| `terminals/create-terminal/create-initial-state` | Accept any string title instead of the requested name. | The terminal title is mutable state; the `cwd` URI check (finding 20) is kept. |
| `terminals/{input,exited,cleared}/*` | Strip OSC escape sequences (shell integration) before matching echoed output. | Terminal data is VT output; the asserted behavior is unchanged. |
| `automations/*` | Wait for the correlated authoritative result instead of a specific echo; skip trigger discovery when the host's advertised `automationsEnabled` root setting is off; skip manual runs when the entry doesn't offer `run`. | `AutomationEntry.operations` is authoritative for what is currently allowed. |
| `telemetry/logs/severity-filtering` | Split out mixed-case level handling into its own test (finding 26). | One behavior per test. |
| `proposed/chat-config/sibling-and-session-isolation` | Take the baseline after creating the sibling chat. | Creating a chat legitimately adds read-only model metadata. |
| `tools/file-tools/*` (all hosts) | `prompts.writeFile` now explains in plain language that the content is the decoded JSON string. | GPT-6 Luna once wrote the JSON literal verbatim. |

## Proposed (post-1.0.0) behavior

Run with `--proposed`, the host negotiates `1.1.0`. Per-chat config (2 tests), fork initial messages,
and initial chat titles pass. Windowed delivery (14 tests) is skipped: subscribe responses carry no
`flowControl`, so the host doesn't implement it yet. The combined steering/queued pending list (3 tests)
is skipped because no `kind`-bearing steering entry was retained in `queuedMessages`.

## Other observations

These are not 1.0.0 violations, but are worth knowing:

- **Automations advertised but disabled.** `InitializeResult.automations` is advertised while the root
  config's `automationsEnabled` setting turns the runtime off, so trigger discovery fails with
  "Automations are disabled" and test-created automations don't offer `run`. Catalogue create, update,
  and remove still work.
- **Git worktree side effects.** When a session's working directory is inside a git repository, the
  `copilotcli` agent isolates the session in a new git worktree and branch (`agents/<title-slug>`, in a
  sibling `<repo>.worktrees` directory) and rewrites the session's `workingDirectories`. After disposal
  the worktrees were removed but the **branches remained**. The runner now refuses scratch directories
  inside a git repository for this reason.
- **Internal errors leak stack traces.** Malformed params (e.g. `subscribe` with no or a non-string
  `channel`) return `-32603` with a JavaScript `TypeError` and stack frames containing local file paths.
- **Extra protocol baseline.** The host also accepts `0.10.0`, a development line.
- **1.1 fields on a 1.0 connection.** Pending messages carry `kind`; harmless to 1.0 clients by itself.
- **Watcher start-up.** A file created immediately after subscribing to a watch can be reported as
  `updated` rather than `added`; changes made once the watcher is delivering events are typed correctly.
- **Variance with a real model.** Whether a tool call needs confirmation depends on the model's chosen
  tool, so some `tools/confirmations/*` tests alternate between passed and skipped across runs.

## Skipped tests

The 92 skips come from proposed features the host doesn't implement (windowed delivery, the combined
pending list), capabilities it doesn't exercise or advertise (`multipleWorkingDirectories.primaryReplacement`,
movable chats, optional RFC 9728 metadata, the deterministic fixture), automations disabled by root config,
optional output the host didn't produce in the observation window (telemetry batches, content references,
reasoning, usage), model-dependent tool confirmations, authentication deferred past session creation, and
optional behaviors the host legitimately chose (unpaginated catalog, replay instead of snapshot recovery).
