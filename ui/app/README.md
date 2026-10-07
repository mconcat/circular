Circular desktop app

The product and the fixture run the same canvas scripts (bootstrap.js canvasScripts). The canvas knows only
the one Source port it is given. The product's port is renderer/adapter.mjs, which projects SDK reads onto
window.STUDY. The mockup's port is fixture-source.mjs, built on the mockup study scripts (fixture.js,
catalog-data.js, time-fixture.js). Gestures reach the port as the closed sum in renderer/gesture.mjs. The
product port lowers them to public declaration verbs, and the mockup port imitates them in its own study.
The canvas scripts never ask which port they are talking to.
There is no product-only HTML file.

For each attachment, main opens one socket and relays frame bytes only (bridge/frames.mjs). It does not read
the bytes. The document opens a public SDK session over those frames itself (`establish` in
renderer/session.mjs), so every value the document holds is exactly what the SDK decoded. A UInt is the SDK's
`CircularUInt`; the app has no carrier type of its own.
preload exposes only `circularConnection` and the frame channel `circularFrames` (send · onFrame · close).
`circularConnection` asks for the attachment result, and asks to reopen a recent project (`{state}`) and to
reattach (`{attach}`).
The two dialogs that open from an action the user clicked also come through this channel (`circular:connection`
in main.mjs).
New project (`{create: true}`) takes a name, creates one 0700 folder, starts a daemon on that folder and attaches
to it. The answer is `{created}` or a refusal code.
Choosing a harness program (`{choose: 'program'}`) returns the chosen file path or null, and binds nothing.

Run npm test in ui/app with the dependencies already installed. Do not run npm install.

The Chrome comparison check does not use Electron or a user profile.
Run the following commands in ui/app. Outputs go under .shots and are not committed.

```sh
node scripts/shot-diff.mjs --write-adapter-fixture .shots/observations.json
node scripts/shot-diff.mjs --chrome '<Chrome executable>' --scene canvas --adapter-fixture .shots/observations.json
node scripts/shot-diff.mjs --chrome '<Chrome executable>' --scene configure --state '<isolated state>' --connection '<SDK connection options JSON>'
```

Create the .shots directory before the first command. Apply the same command to canvas, dense, configure,
approvals and error. Every capture is 1600×1050 with deviceScaleFactor=1.
It writes fixture.png, product.png and diff.png, and prints pixelmatch's changed pixel count, denominator,
ratio and connection reason. The threshold is 0, and anti-aliasing differences count too. Any difference on
the same observed input exits with 1.
The Chrome profile is created inside the output directory and deleted on exit. No real daemon is started.

--adapter-fixture takes one format: {study, catalog, history}. It holds the observed values of the three
original data files and is for comparing the shared presentation scripts on the same data. The mockup's
synthetic time behavior is applied the same way on both sides.
It does not prove that the SDK projection is correct.
There is no format that carries SDK answers as JSON. The product always reads from a session over frames.
This format is test input, not the product's authoring storage format and not an authority.

--state names an isolated state. The connection's socket name and codec ceilings are the values the SDK
publishes (`OWNER_LOCAL_SOCKET_NAME` and `OWNER_LOCAL_RESOURCE_CEILINGS` from `@circular/client`); the app does
not copy those constants. --connection is a test file that swaps those two arguments.
Electron main opens one socket per attachment and closes it on exit. The daemon reads that close as the end of
the session. main has no reconnect loop.
When the session's transport ends or the daemon stops answering, the document keeps its last observation and
says "last observation <recorded time>" and that nothing is being observed. Meanwhile the document asks, once
per second on a UI timer, for the same attachment that Reconnect makes (renderer/attachment.mjs). When the
daemon for the same state comes back (its socket stands and the document's establish succeeds), observation
reopens. The status line and the time machine keep "ended <code> <time> → reattached <time>".
Never run these checks against a real user's Circular state.

The product's actor.events observation learns the end (cut) of each sequence from the first page, reads the
latest window that cut points to once more, and then grows the window with actor.events subscription frames.
The window holds the latest 100 rows and the latest few rows per actor. The number shown on a node is a recorded
coordinate, not the row count in the window: a counter's arrival count is the end of that actor's sequence,
and a timer's emission count is its producer stamp sequence + 1. The state sphere's rate is the number of
arrivals that actor received in the last 60 seconds of recorded time. The journal header counts recorded
arrivals, and the list counts the rows it holds.
Selection reads the selected actor's sequence with the same actor.events query. The local machine row, the
status line and the scope header count one set (the declared actors and their cells) with one classification
(renderer/health-count.mjs).
An arrival on the records subscription is a wake-up to read health again.
The response, alerting and notification views read arrival bodies from the same actor.events window, not
records.fact. Response and alert states are emissions of that actor (records that arrived over a wire), and a
notification is an arrival that another producer sent to an inlet.
A byte body is shown as UTF-8 text, or as its length when it is not text.
When the session's transport ends, the subscription's receive ends with SESSION_CLOSED, and the screen shows
that code and the last observation while it waits to reattach by itself.
New arrival bodies beyond the first page, and restoring a past graph at a recorded cut, need SDK support.
The time machine's screen is the mockup's, and the product does not generate synthetic arrivals.
The time bar draws the daemon's timeline.bins summary (renderer/history.mjs).
The summary has as many bins as the bar is wide; each bin carries the arrival count and the incident count, and
the summary carries four kinds of marker (edit, restart, pause, resume).
The summary walks the whole record, so it is asked only on events: when the view (range, width, lens) changes,
when observation reopens, and when the screen observes a fact that makes a marker (an edit commit, a pause or
resume entry in the execution history, a restart).
The live edge after the end of the summary counts the arrivals the screen received into the summary's bins.
Incidents the screen observes after that (dead letters, failed actors) are not placed in bins; they show as a
count beside the edge. An incident does not ask for a summary.
The next summary replaces both.
The last range option is the whole record, and it is the range shown first.
Scrubbing asks timeline.at once and opens the session's replay lens at the coordinates of the answer
(renderer/replay.mjs).
The cursor shows the time at which that cut stands (resolved_ms), not the time that was asked.
The canvas reads authoring-snapshot at the observation cut and draws its recorded declarations through the
same fold as Live. Returning to Live reads the latest declaration fold again.

Dragging, resizing, connecting, disconnecting, deleting, changing name, flags or view, applying an existing
scalar config, arranging, moving into an existing scope and creating a new container call existing public
declaration verbs from the mockup handlers.
Apply, or the end of a drag, runs BeginEpoch → declarations → ValidateEpoch → CommitEpoch.
After Commit is accepted, the app reads the SDK again and replaces what it shows. It does not retry after a
refusal or an unknown answer.
There is no separate Review/Commit sidebar. The mockup's Apply gesture is used.
Projects → New project creates the state directory selected in the native dialog.
It requests daemon startup through the public CLI.
The app then attempts to attach to that state.
Changing the type or structure of a compound config, editing combinators and injecting data are not
available in this build.
An unsupported gesture is shown with its code, and the mockup's local success is not run.
Undo/Redo (toolbar, ⌘Z/⇧⌘Z, Edit menu) belongs to renderer/undo.mjs. For each accepted edit this session made,
it remembers only the committed values just before the edit and the set of keys the edit named. Undo issues a
new epoch, with the same public declaration verbs, that sets only those keys back to those values.
Keys that another author changed in the meantime, and keys whose target is gone, are left alone and reported
with their code and the verbs that remain.

The normal product start is npm start. Choose the state with the native picker. A launch that names its state
(--state), a fixture or a capture also names its Chromium profile with --user-data-dir: that directory is the
launcher's, and the launcher removes it after the app process has exited. Without it the app opens no window
and exits 1 with the stderr line `profile PROFILE_REQUIRED: …`. For the same
state, File → Reconnect and File → Start Daemon do the same as the window's connection strip: they attach in
place without reloading the document.
The app uses daemon status/start of the circular public CLI that sits next to Circular.app. The daemon starts
only from the viewer's Start daemon, and there is no environment variable default. The CLI and daemon binaries
are not bundled in this build.
The product profile is kept in appData/Circular, so starting the real app must be done separately with an
isolated test procedure.
Development checks do not start the real product app. --fixture and --sdk-fixture are for development.

npm run dist builds the macOS dir/dmg, and npm run dist -- --dir builds only the app directory.
npm run dist -- --check compares the installed SDK copy without building. This README does not cover signing,
notarization, the DMG, or a real installation. These commands do not change SDK files or the engine.
