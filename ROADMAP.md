# uniwidged — Roadmap

Milestones are ordered by **risk first, then by dependency**. The one thing
that could invalidate the architecture is spiked before anything is built on
it; after that the order is whatever unblocks the most.

Each milestone is done when its **exit criterion** holds. "Written" is not an
exit criterion.

---

## M0 · Prove the seam — `wlr-layer-shell` + wgpu ✅ **done, 2026-08-27**

The whole design rests on one unproven claim: that a desktop-layer surface can
back a GPU swapchain with per-pixel alpha. Nobody should write another line of
the framework until that is a running binary.

* Connect to a Wayland display, bind `zwlr_layer_shell_v1`, create a layer
  surface on the `BOTTOM` layer with `keyboard-interactivity` off.
* Drive the configure/`ack_configure` handshake from our own code, then attach
  a `wgpu::Surface` created from `RawWindowHandle::Wayland`.
* Render one Vello scene — a rounded translucent panel with a drop shadow and a
  line of text — and present it.
* Confirm `VK_COMPOSITE_ALPHA_PRE_MULTIPLIED_BIT_KHR`, and that the wallpaper
  is visible through the panel.
* Detect the absence of the layer shell and fall back to `xdg_toplevel`, saying
  so in the log.

**Exit — met.** `spikes/m0_layer_shell/` does all five, with two screenshots and
two arithmetic checks behind them: the wallpaper measurably shows through the
glass (`rgb(25, 43, 72)` where the wallpaper is `rgb(30, 111, 217)`), and an
opaque window covers the widget completely. §10.2 is resolved; §10.1 is
resolved for Wayland and stays open for X11 until M4. Full write-up in
`spikes/m0_layer_shell/FINDINGS.md`.

## M1 · `uniwidged-render` — one device, many surfaces ✅ **done, 2026-08-27**

* One `wgpu::Device` shared by every surface; per-surface swapchain,
  intermediate storage texture and `TextureBlitter`.
* Dirty tracking: clean surfaces are not rendered, not encoded and not
  presented. Every dirty surface's blit goes onto one `CommandEncoder`,
  finished with one `queue.submit`. (Vello 0.9 submits its own compute pass per
  `render_to_texture` call and offers no way to pass in an encoder, so a frame
  costs one submit per *dirty* surface plus one for all the blits together —
  not the single submit originally planned. Measured, not assumed.)
* Two presentation modes behind one interface: a wgpu swapchain, and readback —
  render to a texture, read the pixels back, let the platform composite them.
  Readback is what the X11 mitigation and every headless test use.

**Exit — met.** Twelve tests on a software Vulkan device, including three
surfaces of different sizes rendering different scenes in one `render` call
with each getting its own content, an idle frame costing zero submits, and
undrawn pixels staying fully transparent.

The headless-matches-on-screen half is checked against a real compositor by
`spikes/m0_layer_shell/compare.py`, which renders one scene down both paths and
compares them pixel for pixel: **worst channel difference 1, mean 0.272**. That
comparison found a genuine bug — Vello emits straight alpha where a Wayland
swapchain expects premultiplied, so every translucent pixel was too bright on
screen and nothing looking at one path alone could see it. Written up in
`spikes/m0_layer_shell/FINDINGS.md`.

`vello_cpu` is *not* part of this milestone, and cannot be: see §10.1.

## M2 · `uniwidged-compose` — view tree to pixels ✅ **done, 2026-08-27**

* `View` → taffy layout, with the `gap`/`padding` theme steps resolved to
  pixels.
* Text through Parley: shaping, bidi, fallback, ellipsis, optional wrapping.
* The theme: `Role` → size/weight/opacity, `Accent` → Oklch → the surface's
  colour space. Light, dark and high-contrast.
* Vello scene emission: rounded rects, gradients, real Gaussian shadows, the
  drawn icon set.
* Hit rects and AccessKit nodes emitted from the *same* pass — see
  `ARCHITECTURE.md` §4 on why they may never be computed separately.

**Exit — met.** Fifteen golden images in
`crates/uniwidged-compose/tests/snapshots/`, covering every role, every semantic
accent, the categorical palette, the icon set, both appearances, high contrast,
truncation and wrapping, Arabic in both directions, and 100 %/150 %/200 %
scaling. They render through the whole stack on a software Vulkan device and are
made deterministic by a bundled font, so a colour, weight, radius or shadow that
changes fails CI instead of shipping.

Text is Parley directly rather than through a wrapper: a generic one exposes
only size and a generic family, and two of the §7 commitments need more — font
weight (most of what makes a hierarchy readable, and *all* of it at high
contrast, where opacity is gone) and tabular figures (a clock in proportional
digits changes width as the minute turns).

The box model is taffy behind `layout::boxes`, adopted from Ferrum UI's
`ferrum-layout` and trimmed to what a view can describe. It was a path
dependency on a sibling checkout until 2026-08-28; see `NOTICE.md` on why it is
a copy now, and §"Independent of its parents" below.

72 unit tests besides the images, including the invariant that no colour the
theme can produce falls outside the sRGB gamut — several did on the first
attempt, and a silently clipped palette is one whose author did not choose what
shipped.

## M3 · `uniwidged-runtime` + `uniwidged-guest` — the sandbox ✅ **done, 2026-08-27**

* Generate host and guest bindings from `wit/uniwidged.wit`.
* Wasmtime: instantiate, `ResourceLimiter` memory cap, epoch interruption at
  the call deadline, suspend after repeated overruns.
* Wire the capability policy — already written and tested — to the host
  functions.
* `uniwidged_guest::export!` and the manifest format.
* The conformance widget: attempts every capability, run under deny-all.

**Exit — met.** `widgets/conformance` is a real component built for
`wasm32-wasip2`, instantiated by the real runtime, producing a view the host
accepts. Thirteen conformance tests, of which most are about what is refused:
every capability under a deny-all policy, a `file://` URL refused as *malformed*
rather than ungranted, a path escape refused even where the directory is
granted, and a host grant that does not extend to a name merely containing it.

The suite **builds the widget itself** rather than looking for a prebuilt file:
one that skips when an artefact is missing eventually always skips, and nobody
notices until it matters.

Two findings changed the design. WIT has **no recursive types**, so the view
tree crosses as a validated flat arena — see `ARCHITECTURE.md` §6, where the
ordering rule makes a cycle impossible to express rather than merely detected.
And a **trapped component instance cannot be re-entered**: the first design
counted deadline overruns per call and expected a widget to survive two, and
the second call came back with "cannot enter component instance" in 95
microseconds. A trap now ends the instance, recovery is a fresh one, and
`a_fresh_instance_recovers_a_widget_that_died` proves that path works.

Both halves of containment are tested against a widget that really misbehaves:
one that never returns is interrupted at its deadline, and one that allocates
in a loop is stopped by the memory cap rather than by the clock.

## M4 · X11 ✅ **done, 2026-08-27**

* The window: 32-bit TrueColor visual, `_NET_WM_STATE_BELOW`, `SKIP_TASKBAR`,
  `SKIP_PAGER`, `STICKY`, `WM_HINTS.input = False`, Motif decorations off.
* The `poll` loop, waiting on the display socket and the host's wake pipe with
  a timeout from `Deadline`.
* The three transparency paths of `ARCHITECTURE.md` §10.1, tried in order, with
  the chosen one reported by `uniwidged doctor`.

**Exit — met, with one path deliberately unbuilt.** `spikes/m4_x11` creates the
window through `uniwidged-shell`, renders a widget composed by
`uniwidged-compose`, and checks the arithmetic: the window on screen is the
scene composited over the wallpaper to within one 8-bit unit, and an ordinary
opaque window covers it. Path 3 is forced with `SPIKE_FORCE_READBACK=1` and
produces the same numbers, which is what makes "only presentation falls back" a
measurement rather than a claim.

Path 2 (EGL with an ARGB visual) is **not** built. It would buy GPU presentation
on a driver that refuses alpha through Vulkan, where readback already gives GPU
rasterisation and correct output; that is a performance difference on hardware
nobody has measured. `AlphaPath` still names it, so the gap is visible rather
than forgotten. Write-up in `spikes/m4_x11/FINDINGS.md`.

The screenshots earned their place twice over: they found two rendering bugs
with a single root cause — taffy rounds a computed box down to whole pixels —
that the golden-image suite could not see, because its bundled font made the
arithmetic land differently.

## M5 · Windows

* `CreateWindowEx` with `WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW |
  WS_EX_NOACTIVATE`, `WS_POPUP`; `HWND_BOTTOM` forced in
  `WM_WINDOWPOSCHANGING`.
* A DirectComposition visual tree; wgpu attached via
  `SurfaceTargetUnsafe::CompositionVisual`.
* Per-monitor-v2 DPI; `DWMWA_SYSTEMBACKDROP_TYPE` for the backdrop.
* A `SecretStore` backed by DPAPI, so `secrets` gets the stronger claim: a key
  held by the OS rather than beside the store. The seam is already there.

**Exit:** the widget stays below a maximised window, never steals focus, has no
Alt-Tab entry, and shows the wallpaper through its glass.

## M6 · macOS

* Borderless `NSWindow` subclass with `canBecomeKeyWindow → NO`, level
  `kCGDesktopIconWindowLevel + 1`, `.canJoinAllSpaces | .stationary |
  .ignoresCycle`, `.accessory` activation policy.
* Layer-backed `NSView` with a non-opaque `CAMetalLayer`; wgpu attached via
  `SurfaceTargetUnsafe::CoreAnimationLayer`.
* `NSVisualEffectView` behind it for the backdrop.
* A `SecretStore` backed by the macOS keychain, as M5.

**Exit:** as M5, plus: the widget follows the user across Spaces and does not
slide away under Mission Control.

## M7 · The host and the CLI ✅ **done, 2026-08-28**

* Lifecycle: install, configure, enable, disable, remove; per-widget data
  directory and keychain namespace.
* The capability approval flow — what the user is shown before a widget runs.
* Placement: per-output, saved by output *name*, restored on hotplug.
* Dragging, resizing and a lock switch, from Ephemeris's model.
* `uniwidged run · new · install · list · doctor · snapshot`.
* Autostart per platform.
* The idle soak test of `ARCHITECTURE.md` §5, in CI.

**Done.** `uniwidged-host` is the loop `ARCHITECTURE.md` §5 describes, with real
WASM widgets in it: a supervisor that restarts a failed widget and gives up
after three, damage tracking that draws nothing when a view has not changed,
input routed by hit-testing so a widget is told *which node* rather than where
the pointer was, and a theme change that redraws everything without any view
changing. Twelve integration tests against three real components — including
`widgets/broken`, which hangs on every instance so that giving up can be tested
at all.

**The soak test holds.** Eight widgets installed, first frames drawn, and then
`next_timeout` is `None` and five further ticks draw nothing and run nothing.

`uniwidged-cli` has `run`, `install`, `list`, `doctor` and `snapshot`, and
`run` starts a real desktop on X11: one process, one GPU device, one loop, a
desktop-layer window and swapchain per widget.

### The data directory and the calendar — **done, 2026-08-28**

Two of the gates M3 built now have something behind them, and the shape of the
answer is the point: `Policy` decides whether a widget *may*, and the new
`Services` decides whether this host *can*. A capability nobody granted answers
`not-granted`; one that was granted and has nothing behind it answers
`unavailable`. A widget can act on the difference, and a widget author sent to
fix a manifest that is already right cannot.

* **`fs:<subpath>`.** Each widget gets `<data root>/<its own name>`, created by
  its first write rather than at start-up. Containment is checked **twice** —
  textually, and again after the filesystem has resolved the path, which is the
  only check that can see a symlink somebody planted in a folder a user can
  open. Writes go through a temporary file and a rename, because a widget
  interrupted at its deadline must not leave half a JSON document where its own
  next run will parse one. Per-file size caps in both directions: without the
  read cap, a file somebody else put in a widget's folder is a way to allocate
  a gigabyte in the desktop process.
* **`locale`.** Dates and times in the machine's own UTC offset — through
  `localtime_r`, so it agrees with the rest of the desktop, daylight saving
  included — with the hour cycle and field order from `LC_TIME`/`LANG`. It is
  deliberately not CLDR: numerals only, no month or weekday names, documented
  as such in `services::Locale`. It settles the three things that make a date
  *wrong* rather than merely unidiomatic, and stops there.
* **ABI 1 → 2.** `read-file` answers `ok(none)` for a file that is not there. A
  caching widget's first run finds nothing, and that is its normal case rather
  than its failure case; the previous shape made it read an error string to
  find out.

The conformance widget now reports what a served capability *returned*, not
just that it did not error — a host that answers `format_time` wrongly is not
conformant either. Five new suite tests cover the served path, including that
what a widget writes is on disk in its own directory, that a fresh instance
finds it there, and that the same host which serves `cache/…` still refuses
`../../.ssh`.

### The worker pool, and a flake that was a bug — **done, 2026-08-28**

`Tasks::start` spawned a thread per request. It is called from inside a widget's
`view`, on its sixteen-millisecond call deadline, and spawning a thread is not
bounded work: it reserves a stack and makes a `clone` syscall, and on a loaded
machine that takes longer than a frame. A widget interrupted mid-call while the
host was creating a thread on its behalf is the sandbox punishing a widget for
the host's decision.

It passed every test in `tasks` and failed once — in a full-workspace run, as
"the widget should have produced a view". A trap with no obvious cause, of
exactly the kind that gets re-run, seen to pass, and dismissed as flakiness. It
reproduced only under the whole suite at once, which is the load that made
thread creation slow enough to matter.

The pool is built in `Tasks::new` now, and `start` is a lock, a send, and a
wake — all bounded. Eight workers park on a channel and cost nothing.

The first attempt at that was itself wrong, and worth recording: the workers
held an `Arc<Tasks>`, which is a cycle. A registry its own threads keep alive is
never dropped, so the `Drop` that was meant to stop them could not run — eight
leaked threads per host, and a shutdown path that read correctly and was
unreachable. Splitting the state a worker needs from the registry that owns the
channel is what fixes it: dropping the registry closes the channel, `recv`
returns an error, and the pool goes away. No polling, so an idle desktop still
costs nothing.

### Installing a widget you did not write — **done, 2026-08-28**

M8's second bullet — *a widget registry format, and signature verification on
install* — half of it, and the half that matters. There is no registry, and
this is deliberately not one: a file of keys a user added by hand, one line
each, small enough to be read and audited by the person it protects. Everything
larger needs infrastructure this project does not have and could not honestly
run.

**A signature covers the component and its manifest together.** That is the
whole design, and getting it wrong is the interesting failure. The manifest is
what the user is shown and what the runtime enforces, so a signature over the
`.wasm` alone lets anybody take a signed widget, change `capabilities = []` to
`capabilities = ["net:*.evil.test"]`, and pass it on with a signature that still
verifies — and the user approves a consent screen the author never wrote, on the
strength of a name they recognised. `lifecycle.rs` does exactly that and asserts
it is caught.

The two are length-prefixed into the digest rather than concatenated, or bytes
could be moved from the end of one field to the start of the next without
changing the hash. And the digest is domain-separated, so a signature over a
widget can never be replayed over anything else this project comes to sign.

`install` refuses an unsigned widget by default. `--allow-unsigned` says so out
loud and installs anyway, for the person building their own; it does **not**
override a signature that fails, because a broken signature is a positive claim
that something is wrong where a missing one is only an absence.

Also landed, and all of it named in M7 and never built:

* **`uniwidged check`** — everything `install` would find out, without
  installing: the manifest parses, the component loads, the ABI matches, it
  starts, and it draws *with nothing granted*. Plus the consent lines its users
  will see. What an author runs before publishing and what CI runs on every
  commit.
* **`enable` / `disable`** — a disabled widget keeps its placement, its grants
  and its data. Turning one off to find out whether it was the one making a
  noise should not cost the half-hour somebody spent placing it.
* **`remove`** — and it **keeps the widget's data** unless `--purge` says
  otherwise, because removing a widget is something people do to try one out
  and change their minds. A note-taking widget that silently took the notes
  with it would be a bug nobody could undo.
* **`keygen` / `sign` / `trust` / `untrust` / `trusted`** — the publishing side.
  A secret key is written `0600` and `keygen` refuses to overwrite one.

Thirty tests: eighteen on the signature format and what it must stop, twelve
driving the whole path through the actual binary — because the answer a user
gets is the one the command line prints.

Two crates: `ed25519-dalek` (BSD-3-Clause) and `sha2` (MIT/Apache-2.0), both
already inside the licence policy. Ed25519 because the keys and signatures are
short enough to paste into a README, and pure Rust because a signature check is
the one thing that runs *before* the sandbox does.

### `net` — **done, 2026-08-28**

The capability every document in this repository has been using as its example
since M0, and the last one that needed building rather than binding to a
platform API. The token machinery was already there, so this is a transport —
which is what the previous section predicted, and it is worth saying that it
turned out to be true.

**The whole of it is one rule: a grant for a host is a grant for that host.**
If `api.example.com` answers `302 Location: https://evil.test/`, following it
would turn the grant into "anywhere `api.example.com` cares to point", and the
user who approved one hostname would have approved all of them — without the
widget being complicit, because the *server* decides. So redirects are followed
by `uniwidged-runtime::net` rather than by the client library, and every hop
goes back through the same `Policy` the first request did. A hop off the granted
host stops the chain and says where it was being sent.

The same reading of a URL is used for the check and for the fetch, in one
function, because two parsers disagreeing is exactly the hole this closes.
`https://granted.example@evil.test/` is a URL whose host is `evil.test`; a
reader takes the part before the `@`, and so does a naive parser.

Also refused: plain HTTP, including *via* redirect — a downgrade is carried
through to the check rather than dropped quietly, so the widget is told rather
than seeing a `302` as its answer. There are no cookies, no credentials, no
headers and no method: the interface is a URL, and there is nothing a widget
could be tracked by across other widgets' requests. A response is capped and a
request has a deadline, and both are the desktop's.

**Measured against a real server.** `net/tests` starts a TLS server of its own,
with a certificate chain it generates — a CA and a leaf it signed, because
webpki refuses a CA certificate used as an end-entity and `openssl req -x509`
produces exactly that by default. Sixteen tests: the ordinary fetch, a redirect
within the granted host being followed, a redirect off it being refused, a
downgrade being refused, a loop ending, the body cap, and a `404` arriving as
an answer rather than an error.

**What it cost.** Twenty-six crates, all `Apache-2.0`, `MIT` or `ISC`; `cargo
deny` passes without an exception, which was the condition for taking the
dependency. One licence had to be added to the allow-list — `webpki-roots`,
the Mozilla root bundle, under `CDLA-Permissive-2.0`, which is a *data* licence
for a list of certificates and is permissive. Allowed explicitly rather than by
loosening the policy.

Two things `cargo deny` caught that a person would not have: that licence, and
`rustls-pemfile` being unmaintained since August 2025 — a thin wrapper over PEM
parsing that `rustls-pki-types` has included since 1.9. The tests use the real
thing now.

### Asynchronous capabilities, and `command` — **done, 2026-08-28**

The mechanism `fetch` was designed around in M3 had never once run. `Event::
Delivered` was a variant nothing produced, `take-response` a function nothing
called, and the whole shape — ask, get a token, be told later — existed only as
a claim in the WIT file. It runs now, and `command` is what proves it.

**One mechanism, not one per capability.** `fetch` and `run-command` both start
a task; `take-result` collects any of them and says what kind of answer it is.
Adding a third kind is a variant, not a mechanism. That is the ABI 3 → 4 change,
and it is why `net` will be a transport rather than a redesign.

**Tokens are owned.** They come from one counter shared by every widget on the
desktop, so nothing stops a widget guessing its neighbour's — except that the
handle a widget is given has its own id baked in, so no host function can start
or collect a task without saying whose it is. A token that is not yours reads
exactly like one that never existed, because distinguishing them would report
how many requests the neighbours have made.

**The desktop never waits.** Work happens on a worker thread against a
desktop-wide budget of eight, with a per-widget limit of sixteen outstanding —
a widget that starts a request on every view is *refused*, which is the host
telling its author about the bug rather than quietly growing. And a finished
task has to be able to end a block that has **no timeout at all**, which is what
`uniwidged_shell::wake` is: a pipe in the loop's `poll` set beside the display
socket. `ARCHITECTURE.md` §5 named "the host's wake pipe" from the start; this
is it, built.

**What `command:<name>` costs.** It is a hole a user deliberately opens, so the
tests are about how wide it is: no shell anywhere — the injection everybody
expects is asserted against directly — the granted name exactly and never a
path, `PATH` and nothing else in the environment because a desktop's holds
session tokens, no standard input, and bounds on time and output enforced by
killing rather than asking. Output is read on its own threads, because waiting
for a process and reading afterwards deadlocks the moment it fills a 64 KB pipe
buffer, which happens on real output rather than only in theory.

The consent line still says "run the `uptime` command", and that is honest as
far as it goes: a command can do whatever that command can do, and neither the
host nor the user can know what that is. `command:sh` is arbitrary code
execution wearing a small word. The guide says so in as many words rather than
making it look small.

**A fourth reference widget.** [`widgets/uptime`](https://github.com/JosunLP/uniwidged/tree/main/widgets/uptime) is the
asynchronous shape written out — ask, draw something now, collect on the event —
and it asks for `uptime` rather than `sh` for the reason above. Its test watches
the first frame say it is waiting and a later one carry the machine's real
uptime.

### Wayland — **done, 2026-08-28**

The platform half of the project's promise, and the one a majority of Linux
users are actually on. `uniwidged run` now creates a `zwlr_layer_shell_v1`
surface on the `BOTTOM` layer, and the M0 spike's code has been promoted into
`uniwidged-shell` rather than left where only a person running a script could
reach it.

Everything above the shell is unchanged, so all three differences live in
`platform/wayland.rs`:

* **A client cannot place a window.** There is no `ConfigureWindow`. A layer
  surface is anchored to an edge and pushed away by margins, so `set_geometry`
  is an anchor and two margins — the same result by a different mechanism.
* **A client is never told where the pointer is.** Wayland reports
  surface-local coordinates only, deliberately, so a client cannot watch the
  pointer cross somebody else's window. The desktop coordinates the interaction
  model wants are therefore *computed* from the placement this file asked for,
  which it knows exactly because it set it. The host's drag logic is then
  identical on both platforms, which is what stops a widget behaving
  differently depending on which session it was started in.
* **The pointer grab is implicit.** From button-down to button-up every event
  goes to the surface it started on. X11 needs `grab_pointer` for the same
  guarantee; here asking for it would be asking for something that does not
  exist.

`platform::linux` is where the two meet: one `DesktopWindow`, chosen at run
time, because a user does not rebuild uniwidged when they log into a different
session. A Wayland session that also runs XWayland gets the native protocol —
going through XWayland means giving up the layer shell, which is the whole
definition of a desktop widget. If the Wayland connection fails, X11 is tried
anyway and the start-up log says which happened.

libwayland is loaded with **`dlopen` rather than linked**. One binary then runs
on an X11-only machine that has no libwayland at all — which is most servers
and every container this gets built in — and the build needs no `-dev` package.

**Measured.** `crates/uniwidged-shell/tests/wayland_layer.rs` starts its own
headless `sway`, creates the surface `uniwidged run` creates, and checks the
facts only a compositor can confirm: that the layer shell accepted the `BOTTOM`
role, that the configure handshake completed with a size the *compositor*
chose, that a placement survives a round trip through it, that the monitor comes
back named, and that an idle widget actually blocks. The `xdg_toplevel`
fallback — what a GNOME session gets, since Mutter has no layer shell and has
said it will not — is exercised deliberately by a parameter rather than an
environment variable, because a test that has to set one cannot run beside
another test.

One finding worth keeping: sway puts its IPC socket at
`$XDG_RUNTIME_DIR/sway-ipc.<uid>.<pid>.sock`, and a Unix socket path cannot
exceed 108 bytes. Past that sway logs one line and segfaults, which from a
client looks exactly like "no compositor" — and `TMPDIR` on a build machine is
routinely long enough to trigger it. The test's runtime directory is short by
construction now, and says why.

**Not done here:** pointer input is wired and compiles on Wayland but is not
measured — headless sway has no input device, and driving one through sway's
IPC is a test this does not have yet. The X11 pointer tests cover the shared
interaction model; what is unverified is the Wayland event plumbing into it.

### The fonts and the machine — **done, 2026-08-28**

The two remaining capabilities that could be served without a network stack or
a platform keychain, which leaves `net`, `secrets` and `command` as the whole
of what is still `unavailable`.

* **`measure`.** A widget asks how wide its own label would be, in a named
  role, and gets the number the layout will actually arrive at — same fonts,
  same size, same weight, same tabular figures. `Compositor::measurer()` hands
  out a handle that *shares* the compositor's font context and theme rather
  than copying them, so a theme change reaches a widget's measurements as well
  as its pixels. A copy would keep answering, plausibly, with the old number,
  and `desktop.rs` has the test that would catch it.

  Two paths now compute a string's width — the measure callback in
  `layout::compute` and `Measurer::width` — and `layout::tests` asserts they
  agree, including that both consult `is_numeric`. Tabular figures change every
  digit's advance, so a widget measuring one thing and being drawn as another
  is exactly what that would look like.

* **`metrics`.** `cpu_load` from `/proc/stat`, one sampler shared by the whole
  desktop because it is one machine. It **never blocks**: the obvious
  implementation sleeps between two reads, and a host function that sleeps is a
  widget holding its deadline open inside `view()`. So the first call answers
  with the average since boot — a real number, and the only one available — and
  each call after it with the window since the last sample, with anything
  asked inside 250 ms getting the previous answer rather than noise. `guest`
  and `guest_nice` are deliberately excluded from the total: they are already
  inside `user` and `nice`, and adding them is how a machine running virtual
  machines comes to report 120 % load. There is a test for that specifically.

* **ABI 2 → 3.** `measure-text` takes the `role` enum rather than a `u8` index
  into it. The index had no meaning a widget author could look up and nothing
  stopped one passing 200.

**One thing had to change to make it possible.** Wasmtime requires a store's
data to be `Send`, and the shaping cache held Parley layouts behind `Rc`. So
`Shaped` is `Arc` now and the compositor shares its fonts and theme through
`Arc<Mutex<_>>`. Nothing crosses a thread — there is one loop — and the locks
are never contended; what the change buys is that the claim is checked by the
compiler rather than asserted by a comment. The alternative was an
`unsafe impl Send` over an `Rc`, which is unsound the moment it stops being
true and gives no warning when it does.

### Moving a widget — **done, 2026-08-28**

Dragging, resizing, the lock switch and placement by output name, which is what
the milestone's exit criterion was really asking for.

**The rule for what a press means** is in `uniwidged-host::interaction`, and it
is the whole design: a press within six logical pixels of an edge resizes; a
press on a node the widget marked interactive is the widget's; anything else
moves it. The second of those needs to know which nodes those are, so
`uniwidged-compose` records them in the same pass that records the hit
rectangles — the argument in `ARCHITECTURE.md` §4, applied again.

Nothing happens until the pointer has travelled four logical pixels, which is
why a click is delivered on *release*: before that, there is no way to tell the
two apart. Nineteen tests cover the ambiguities — a press dragged off a button
is neither a click nor a move, a locked widget still takes clicks, the corner
that grows a widget leaves the opposite corner exactly where it was, and a
widget cannot be resized inside out or dragged out of reach.

**Placement is stored relative to a named output.** That is what makes a
position survive docking: indices renumber, and desktop coordinates move
wholesale when a screen is plugged in on the left. A widget whose monitor is
gone appears on the primary **and keeps its stored name**, so plugging the
monitor back in puts it back — rewriting the name would have made the first
undock permanent. Eleven tests, including that placing and recording are
inverses, which is the property a sign error in either direction survives
everything else.

**Measured, not asserted.** `crates/uniwidged-shell/tests/x11_input.rs` starts
its own nested X server, opens a real desktop-layer window, synthesises pointer
input through `XTest`, and reads the geometry back **from the server** — a drag
computed from window-relative coordinates passes every arithmetic test and
shivers on screen, and a missing pointer grab works perfectly until the pointer
leaves the window. Neither is visible without a server. Seven tests, in CI,
with `UNIWIDGED_REQUIRE_X11=1` so a runner without `Xvfb` fails rather than
skips.

Two findings came out of running it. Tests that each picked an unused display
number picked the *same* one and saw each other's pointer — the symptom was an
assertion about a resize reporting a move, by exactly the distance a different
test had dragged. And an X server **resets when its last client disconnects**,
which refused the connection immediately after the one that proved the server
was up; the fix is to keep a client attached for as long as the server is
wanted. Both are in the file, next to the code that guards against them.

`uniwidged lock`, `unlock` and `move` do the same from the command line,
because a pointer is not always available: a widget can be placed over SSH,
from a dotfiles script, or back onto a monitor that was unplugged while it was
there.

**Not done, and named rather than glossed over:**

* ~~**The keychain namespace.**~~ Done — see below.
* ~~**A disk quota.**~~ Done — see below.
* ~~**Autostart.**~~ Done — see below.
* ~~**Two monitors, actually tested.**~~ Done — see below, against two virtual
  RandR monitors on a server the test starts. What remains untested is two
  *physical* monitors, which this machine does not have and which differ from
  the above only in the hardware behind the same protocol.
* ~~**Pointer input on Wayland.**~~ Done — see below, against a virtual pointer
  device the test creates, because headless sway has no real one.
* ~~**Applying a configuration without a restart.**~~ Done — see below.
* ~~**Installing and removing without a restart.**~~ Done — see below.

The milestone's original exit criterion — eight widgets across two monitors
surviving a restart — is met except for two *physical* monitors, which this
machine does not have; two virtual RandR ones are tested against a server the
suite starts. Everything that criterion stood in for holds: a desktop full of
real sandboxed widgets runs, draws, responds, can be moved and resized and
pinned, remembers where it was put, costs nothing at rest — and can be filled
and emptied while it is doing it, which the criterion did not think to ask for
because restarting was assumed.

### The keychain, the quota, and starting with the session — **done, 2026-08-28**

The last three things M7 named and did not build. Two of them turned out to be
one paragraph of code and one page of reasoning, which is the ratio this
project keeps producing; the third found a bug in the first version of itself.

#### `secrets` — the last gate with nothing behind it

It was right to be last. Every other capability degrades harmlessly when it is
missing: a widget without `net` shows stale data, one without `metrics` shows a
dash. A keychain is the one where *having* it badly is worse than not having
it, because a token written somewhere readable has already leaked and nothing
later undoes that.

So what landed is a seam rather than an implementation. `SecretStore` is the
trait, `Keyring` is the handle a widget is served through — carrying its
namespace exactly as `TaskHandle` carries its id, so there is no argument
through which a host function could reach somebody else's secrets. Behind it,
today, one store: **one file, encrypted whole with XChaCha20-Poly1305**, under
a key in a second file at mode `0600`.

**The store is one message, not one per entry**, and that is the interesting
decision. A per-entry format leaks the list of key names to anybody holding the
file, and the names are frequently the whole story — `github.token` says more
than its ciphertext does. It also makes editing detectable: change a byte and
the file does not open, rather than decrypting to something a widget believes.

**And the description is part of the feature.** "Store passwords in your
keychain" is a sentence whose weight depends entirely on what the keychain is,
so `SecretStore::describe` returns the honest one — *safe from a backup or
another user, not from a program already running as you* — and `install` prints
it at the moment somebody is asked to approve it, not only in `doctor` where
nobody looks first. A session keychain can make the stronger claim, because its
key is held by a daemon a login unlocked; those are backends behind the same
trait and are named in M5 and M6.

**The bug worth recording.** The first version called `fsync` before the
rename, which reads as the careful choice and is not: these functions run
inside a widget's `view`, on its sixteen-millisecond deadline, and an `fsync`
waits on a device. Under a full-workspace test run it interrupted widgets for
the crime of storing a token — the sandbox punishing a widget for something the
host chose to do on its behalf, which is precisely the mistake `Tasks::start`
made by spawning a thread per request, three sections above. The atomic rename
is what actually protects the store and it is kept: a power cut can cost the
most recent secret, and cannot leave a keychain that will not open. Reads were
the other half — the store is opened once and cached, because re-decrypting a
file on every frame is the same mistake in slower motion. Both are asserted:
a hundred reads must fit inside one frame, which they cannot if any of them
touches a disk.

**ABI 4 → 5.** `get-secret` answers `ok(none)` for a key that was never set.
This is the 1 → 2 change made a second time, in the one other place where
absence was being reported as an error — a widget's first run has no token, and
that is its normal case. `remove-secret` is new: a widget that cannot forget a
credential is one that keeps it forever, which for a credential is a bug rather
than a missing convenience.

**A fifth reference widget.** [`widgets/vault`](https://github.com/JosunLP/uniwidged/tree/main/widgets/vault) is the shape
every widget that signs in to something ends up with — it has a credential or
it does not, it says which, and it can forget one. It declares `secrets` and
nothing else, and two of its tests are about what it must *not* do: the
credential never reaches the view, which is composed and handed to an
accessibility tree, and it is not on disk in the clear.

Twenty-one tests on the keychain itself, of which the file backend's half is
the interesting one — that the secret and the key name are not in the file,
that a store somebody edited refuses to open, that a store restored without its
key says so rather than reporting an empty keychain, and that a key file
anybody can read stops the whole thing instead of making the encryption
decorative. Plus the conformance widget's whole lifecycle — absent, holding,
absent again — which a host that quietly kept the old value would fail where a
write-then-read would pass.

One crate: `chacha20poly1305` (MIT/Apache-2.0), inside the licence policy,
`cargo deny` unchanged.

**Not done, and deliberately.** The Secret Service — gnome-keyring, KWallet —
is the Linux backend that would make the stronger claim, and it is a D-Bus
client: a session bus connection, an `org.freedesktop.secrets` collection, a
prompt when the keyring is locked. Two things make it more than an afternoon.
It is not universally present, so the file store stays either way and the
choice has to be reported rather than assumed. And **a prompt is unbounded** —
a locked keyring can put a dialog in front of the user, and a `get_secret`
inside `view` cannot wait for one. That is the same argument as the `fsync`
above and has a real answer — the host opens the collection once, at start-up,
off any widget's deadline — but it is a design rather than a binding, and doing
it badly would be worse than the honest file.

#### A disk quota

Named as missing for as long as it took somebody to write the loop. The
per-file cap bounds what one host call can allocate; this bounds what a widget
can do with a `for`.

Two things it has to get right, and only one of them is the refusal. The other
is that a widget rewriting its own cache file at full size must **not** be
refused — that is the ordinary behaviour of exactly the widget this exists to
bound, so checking the directory as it stands would break every honest widget
and leave the loop working. The check is against what the write *replaces*.

Symlinks are not followed and their targets are not counted, in either
direction: a user who symlinks something into a widget's folder has not handed
the widget a bill for it, and a widget cannot make its own usage look smaller
by pointing somewhere empty. And a full quota answers `unavailable` where an
oversized file answers `malformed`, because the two have different fixes — one
is a bug in that call, and the other is a well-formed request the host cannot
serve until something is deleted.

#### Autostart

`uniwidged autostart enable | disable | status`. An XDG desktop entry, because
that is the one mechanism GNOME, KDE, XFCE, Cinnamon, MATE, LXQt and every
tiling compositor that bothers all read — and the one a user already knows how
to find. A systemd user unit would work on the subset of those running systemd
`--user` and would leave the widget host as the one thing in somebody's session
they cannot turn off from their settings panel.

Most of it is about what it will not do. The entry carries
`X-Uniwidged-Managed=true`, and both `enable` and `disable` **stop** on a file
without it: somebody who hand-wrote an autostart entry with their own wrapper
script in it has said something, and replacing or deleting it would be this
program deciding it knows better. `status` exists for the third state, which is
the one that is impossible to debug from the symptom — an entry that is fine
and points at a binary that has been moved, upgraded or `cargo clean`ed. At
login that is silence.

Seven tests, driven through the actual binary against a session directory of
their own, because the answer a user gets is the one the command line prints.

#### And a Wayland flake that was a test, not a bug

`an_idle_widget_waits_without_spinning` asserted that one `wait` blocks. It
failed once under a full-workspace run and passed alone — the shape that gets
re-run and dismissed. It was the test: a single wait can return early and be
perfectly correct, because the compositor sent the client something and `poll`
reported the socket readable, which is the mechanism working. What would be a
busy loop is that happening repeatedly with nothing behind it, so that is what
is asserted now. Worth separating from the `Tasks` flake three sections above,
which looked identical and was a real bug: the difference is whether the thing
being sampled is allowed to vary.

### Configuration, and two monitors — **done, 2026-08-28**

Two more of M7's bullets, and the first of them closes a gap that had been
sitting in the interface file since the very first commit.

#### The event that had never fired

`Event::Configured` has been in `wit/uniwidged.wit` since the first version and
nothing has ever produced it. Not because it was hard, but because there was
nothing to configure *by*: a widget had no way to declare a knob, a user had no
way to turn one, and a variant with nothing behind it is a claim rather than a
feature. That is exactly the shape `Event::Delivered` was in before the
asynchronous work, and it is closed the same way — by building the thing the
variant was waiting for.

**A widget declares; the host validates; the widget reads.**

```toml
[[setting]]
key = "warn-at"
label = "Warn above"
type = "number"
default = 0.6
min = 0.0
max = 1.0
```

Four kinds — `text`, `number`, `toggle`, `choice` — and no fifth, for the same
reason the icon set is closed: what a host cannot render is not something a
widget should be able to ask for.

**The point of declaring rather than handing over a string map** is that a
widget can never be given a value it cannot handle. The type, the range and the
option list are checked before a widget is started, so `cx.setting("warn-at")`
returns a number or nothing at all and the code reading it has no branch for
somebody typing `banana`. The user was told at the moment they typed it, by
`uniwidged configure`, which is where a refusal can still be acted on.

**The default is checked at parse time**, against the setting's own rules. A
manifest shipping `default = 2` for a value capped at 1 has a number its own
form would never let a user re-enter, and nothing would notice until somebody
opened the form. `uniwidged check` shows the whole form, so an author finds out
before publishing rather than after.

**A stale key is reported, not fatal**, and the asymmetry with an over-grant is
the interesting part. An over-grant refuses: the file exceeds what the user
approved, and obliging it would hand a widget something nobody agreed to. A
stale setting is what an *upgrade* produces — a widget dropped a knob in its
new version — and refusing to start the desktop over that would make every
upgrade a support question. So the key is ignored, the default is used, and
`uniwidged configure` and the start-up log both say which line is being
skipped. Nothing is rewritten: a desktop that silently deletes a line somebody
typed is worse than one that explains why it is ignoring it.

**Only what differs from the default is written.** Recording a value equal to
the default would pin *this version's* default into the file, so a widget that
later changed its mind about a sensible starting point would never reach the
people who never touched it.

**Not a capability.** There is no argument through which to name another
widget's settings, the values came from this widget's own manifest, and a user
who declined them would be declining the widget rather than a permission. `log`
is the other host function with the same shape.

**The handle is shared, not copied**, so a change reaches a widget that is
already running. Reconfiguring by restarting would throw away everything a
widget had loaded in order to alter a number — and for a widget holding a
credential it would mean signing in again to move a threshold.

ABI 5 → 6. Fifty-odd tests: the schema and what it refuses, the manifest
parser, the resolution of a file against a manifest, the whole path through a
real component, the desktop applying it to a running widget, and twelve driving
`uniwidged configure` through the actual binary. `widgets/system-load` is the
worked example — the two thresholds are the numbers a widget cannot pick for
its user, because on a laptop 60 % is something to notice and on a build
machine it is Tuesday — and `uniwidged new` now scaffolds a setting, because a
feature a new author has to find in the guide is a feature most of them do not.

#### Two monitors, actually tested

Named as missing for two milestones: the placement logic was tested against a
docked-laptop layout in memory, and the enumeration against a nested server
with one screen. Nobody had watched the two meet, and everything about
placement is arithmetic that is invisible on a single screen whose origin is
zero.

`xrandr --setmonitor` is what makes it testable at all. A nested `Xvfb` has one
screen and no real outputs, and RandR 1.5 lets a client declare monitors over a
screen's geometry — which is what `randr_get_monitors` then reports and what
`uniwidged-shell` reads. So the code under test sees two named monitors side by
side, through the same protocol path a docked laptop uses.

Four tests: both monitors enumerated by name and position, a widget stored
against the right-hand one opening at the right desktop coordinates *read back
from the server*, an unplug that moves the widget somewhere reachable while
keeping its stored name — and a re-plug that puts it back exactly — and a drag
across the seam recorded against the monitor it landed on.

One finding, and it is the kind this exists for: **no monitor is flagged
primary** on such a server. X reports one only when something has set it, and a
fresh RandR configuration or more than one Wayland compositor will not have.
`placement::primary` already fell back to the first output rather than indexing
into nothing, so this is a path that was written correctly and had never been
run.

#### And a Wayland flake that really was a bug

`a_placement_survives_a_round_trip_through_the_compositor` failed once under a
full-workspace run, reporting the size the surface started with. Not the
machine: `WaylandWindow::take_resize` compared the compositor's last known size
against the size the client had just *requested*, so in the gap between
`set_geometry` and the answering configure it read the client's own pending
request as a compositor resize — and reverted `placement.rect` to the old size,
after which nothing asked again.

On a quiet machine the configure arrives inside the same drain and the gap is
invisible. On a loaded one it is not, and on a real desktop this is a widget
that is dragged or resized on Wayland and snaps back. The comparison is against
what was last *reported* now, which is a different fact from what was last
requested, and there is a test that asks `take_resize` in the gap directly
rather than leaving it to a race.

Worth separating from the two flakes above it, which were tests: this one was
found by a test that was also wrong, and fixing only the test would have hidden
it.

#### And a flake that was a fixed sleep

`a_widget_dragged_across_the_desktop_is_recorded_against_the_monitor_it_lands_on`
failed once, under load, with `placed: None` and nothing to suggest why: the
button release arrived after the fixed 120 ms window the test spent pumping
events. The end of a gesture is a fact, not a duration, so it is waited for now
— on the *release* having been processed rather than on a placement, because a
release that produced neither a click nor a placement is a real answer and this
file has three tests about it. The suite got faster, which is the usual sign
that a sleep was in the wrong place.

### Telling a running desktop, and links that open — **done, 2026-08-28**

The previous section shipped configuration and ended with a bullet naming what
was missing: *a way for one process to tell the other, and the honest options
are a signal and a pid file or a socket. Named rather than half-built.* This is
that, plus the other thing the API reference had to admit was a no-op.

#### `SIGHUP` means read your configuration again

A socket would be a wire format, a version, a permission model and a second
thing to keep working. What is actually being sent is one bit, and `SIGHUP` has
meant exactly it to a daemon for forty years.

It costs the loop nothing. The handler does two things, both async-signal-safe:
a relaxed store, and one non-blocking write to the wake pipe that has existed
since the asynchronous work for finished tasks. An idle desktop still blocks on
its window system's socket with no timer at all.

**The dangerous part is the pid, not the signal**, and it is worth being plain
about. `SIGHUP`'s default action is *terminate*. A pid file left behind by a
crash names a number the kernel will happily reuse, so signalling it on the
strength of the file is a way to kill somebody's editor. The pid is checked
twice: that the process exists, and that it is this program, by reading
`/proc/<pid>/comm`. A pid that is gone or is something else is stale, and stale
means ignored. There is a test whose comment says what would happen if the
check were wrong — the suite would send `SIGHUP` to its own harness, which
would not fail, it would take the run with it.

**What can change while a desktop runs is decided by what can be done
atomically.** Settings can, because they are values behind a shared handle —
which is why `Host::configure` was built to reach a running widget rather than
to restart one. Placement and the lock switch can, by the layer that owns
windows. Installing and removing cannot: a widget arriving needs a component
compiled, a window created, a swapchain attached and a placement resolved, and
doing half of that on a signal is how a desktop ends up with a window nothing
draws into. Those are reported, the user is told it takes a restart, and
everything already running keeps working.

Every command that writes the configuration now nudges a running desktop, and
says so in one line when there was one to nudge. `uniwidged reload` is the same
thing for a file edited by hand.

**And running it for real found the bug the unit tests could not.** The first
version reported "nothing changed" over a setting it had just applied. The
cause was one field doing two jobs: `configure` returned whether *the widget
said its view would differ*, and the caller read that as whether *anything was
applied*. They are different facts, and `system-load` is the widget that shows
it — moving a warning threshold past nothing changes no colour, so it correctly
answers that its view is unchanged while its configuration has very much
changed. Two fields now, `applied` and `redraws`, and a reference-widget test
that pins the distinction.

The whole path was driven end to end against a real desktop on a nested X
server: `configure`, `move`, `lock` and a bare `reload` each producing exactly
the line they should, and the pid file removed on a clean exit.

#### Links, which turned out to be a different question from `net`

`.link(url)` has been in the view vocabulary since M2. An assistive technology
was told a node was a link, and **nothing opened it** — a fact discovered while
writing the API reference and written down there rather than implied away,
because handing a widget a way to make the host launch an arbitrary URL is a
capability and had not been designed.

It is `link:<host>`, and the interesting decision is that it is *not* `net:`.
They read alike and are different sentences: `net:api.example.com` is "this
widget talks to a server", `link:example.com` is "this widget can put something
on your screen that takes you to a web page". A widget may want either without
the other — one that reads a local sensor and links to a forecast has no
network grant at all — and a user may reasonably say yes to one and no to the
other. Neither implies the other, and the two matchers are separate functions
so a wildcard cannot come to mean one thing across both.

**`https` only, checked before the grant is.** `file://` opens a file manager,
and a registered `x-scheme-handler` opens whatever somebody installed, so a
system opener handed an arbitrary URL is a widget running arbitrary programs by
another name. The URL is read by the *same* function `net` reads one with —
see below, because that function had quietly become two.

The host decides whether; the caller opens. Launching a browser is launching a
process, and the host does not launch processes any more than it owns windows —
which is also what makes the whole thing testable with no browser. A widget may
draw a link it was not granted; it does not open, the desktop says why, and the
widget is told about the click either way. Withholding the *news* would leave
it unable to explain itself.

#### And the parser that had quietly become two

`net::host_of` carries a comment saying it lives there so that the two readings
cannot drift: *a redirect judged against one parse and fetched against another
is exactly the hole this file exists to close.* There was a second, identical
copy in `instance.rs` — the file that performs the **first** check. That is the
hole with the warning written above it.

They happened to agree, which is not a property anybody was maintaining. There
is one now, and the link check uses it too, so a third reading cannot appear
without deleting the same comment twice.

### Widgets that arrive and leave while the desktop is up — **done, 2026-08-28**

The last thing M7 named and refused, and the reason it refused was good: a
widget arriving needs a component compiled, a window created, a swapchain
attached and a placement resolved, and doing half of that on a signal is how a
desktop ends up with a window nothing draws into. Until now the answer was
`uniwidged install` writing a file and printing *it starts when the desktop
does*, which is a sentence somebody reads once before deciding this is not a
program you install widgets into.

What changed is not the risk. It is that **each half is now one function that
does all of it or none of it**, and both live in the only layer that owns
windows.

**The order inside a removal is the whole of its correctness.** `Host::remove`
detaches the renderer's surface first, because while a surface is attached the
next frame composes into it; the window is destroyed second, because dropping
it destroys what wgpu was presenting to. Reversed, the renderer holds a surface
over a window the X server has already forgotten — which is not a type error,
not a panic, and not something any headless test can see.

**Two things go with the widget, and one deliberately does not.** Its schedule
goes, or the loop wakes once a second for a widget nobody can see. Its
outstanding asynchronous work goes: ids come from a counter, so a `fetch` left
in flight is an answer the *next* widget installed would collect — the same
reasoning, and the same `Tasks::forget` call, that a suspension already made.
What stays is everything the widget wrote. Removing a widget from a running
desktop is not a decision to destroy what it saved; `uniwidged remove --purge`
is where that is asked for, and it asks.

**`reload::apply` still refuses to do either**, and that is the same split
`ARCHITECTURE.md` §5 makes about surfaces rather than a leftover. It reports
which names appeared and which disappeared, and every widget it names is *still
running* when it returns. The caller — `uniwidged_cli::run`, which owns the
windows — is what acts. A test asserts the widget is still there afterwards,
because that contract is what makes the two halves separable at all.

**Removals happen before additions**, which is not tidiness: a widget upgraded
in place is a removal and an addition of the same name, and the other order
would have two windows with the same title and two instances writing the same
data directory for as long as it took.

And an upgrade *is* now applied live, which is the case none of this would have
covered otherwise. `uniwidged install` over an existing widget, and every
`uniwidged update`, leave the name in the file and change what is underneath
it — invisible to a diff of names. So a reload asks a second question of every
widget still in the file: is the component path the one it was started from,
and is the version beside it the version it is running? A widget that answers
no is stopped and started, keeping what the file keeps — where it sits, what it
was configured with, whether it is switched off.

**Measured against a desktop that is actually running.**
`crates/uniwidged-cli/tests/live.rs` starts the real binary on a real `Xvfb`
with a real device, puts a real WASM widget on the screen, and then — from a
second process, through the pid file, over `SIGHUP` — installs a second widget,
removes it, disables one and enables it again. Three tests, and the third is
the one that would have been left out: a desktop whose *last* widget is removed
has no window to wait on, still has to block rather than spin, and has to
accept a widget again afterwards.

### The other end of the wait — **done, 2026-08-28**

Two bugs, both found by reading the wait path rather than by any failure, and
both invisible to every test in the repository — because every one of those
drives a loop that is already awake. What was wrong was what happens on the way
*into* a block.

**A desktop waited on one widget's connection.** Every window carries its own:
`XCBConnection::connect` per X11 window, a `wl_display` per Wayland one,
because a window is opened by whoever wants one and neither protocol has a
session object above them. The loop called `wait_with` on `windows[0]`. On an
idle desktop `next_timeout` answers `None` — the property the entire wake model
is built around — so the process blocked with no timer at all, and a click on
the second widget sat unread on the second connection until something else
happened to wake it. On a desktop of two clocks that is up to a second. On a
desktop of two static widgets it is until the user gives up.

`platform::linux::wait_any` polls every connection and the wake pipe in one
call. `a_press_on_the_second_widget_wakes_a_desktop_that_is_blocked` blocks
with no timeout at all, exactly as a desktop does, and would hang without it.

**And the check that existed to stop an event being slept through threw it
away.** Before sleeping, the X11 path asked whether anything was already
queued — an event that arrived between the last drain and the call is in a
queue, not on a socket, and no amount of polling will report it. It asked with
`poll_for_event`, which *removes* what it returns. So the press was consumed
and discarded, the release that followed reached an interaction model that had
never seen a press, and the click did nothing. Once in a while, for no reason
a user could describe. There is a pushback queue now, and `has_queued` is the
question both the single-window path and `wait_any` ask.

### The monitors, while the desktop is running — **done, 2026-08-28**

M7 lists *placement restored on hotplug*, and that had always been half true in
a way worth being precise about: `uniwidged_host::placement` could put a widget
back on the monitor it was stored against, and did — at start-up, and on a
reload, and at no other time. Nothing was watching. So a laptop docked while a
desktop was running left every widget at coordinates belonging to a layout that
had stopped existing, and undocking one left them off the screen; the placement
was correct and nobody had asked for it.

Both window systems can say. X11 has a RandR event mask on the root window and
Wayland announces and withdraws `wl_output` globals, and both now arrive as one
`Input::OutputsChanged` — a bare fact carrying no list, deliberately. Each
reports a layout **one piece at a time**: a laptop being docked is several
global announcements followed by several rounds of geometry and mode, and a
desktop that acted on each would place every widget against a layout the
compositor had already moved on from. So it means *ask again*, and the loop
asks once, however many notifications arrived, and re-resolves every placement
against the answer.

**Four RandR masks rather than one, and that is a measurement.** A monitor
declared or withdrawn produces an `RRNotify` with a resource-change subtype and
no screen change at all, because the screen's own size has not moved; a cable
produces the output and CRTC ones. Selecting only `SCREEN_CHANGE` reports some
of the ways a layout can change and not the others.

**A session that answers with nothing is left alone.** An X server without
RandR or a compositor advertising `wl_output` below version 4 cannot say what
its monitors are called, and the right response to an empty list is to leave
every widget exactly where it is. Wiping the list would move all of them to the
origin — which is the failure mode this feature exists to prevent, arrived at
from the other direction.

**Measured, and the measurement found something first.** `xrandr --setmonitor`
— the command the two-monitor tests already used — declares a RandR 1.5 monitor
and notifies **nobody**. The first version of the test used it and failed
against a working implementation, which took a probe over the raw connection to
tell apart. What `Xvfb` will actually do is turn its one output off and on, and
that is what
`a_monitor_going_away_and_coming_back_reaches_the_desktop` does.

### Pointer input on Wayland, measured — **done, 2026-08-28**

The last of M7's named gaps. `platform::wayland` binds a seat, takes its
`wl_pointer` and turns enter/motion/button into the same `Input` values X11
produces — and nothing had ever seen one arrive, because headless sway has no
mouse and there was nothing to move.

There is a way, and it is the Wayland spelling of what the X11 suite does with
`XTest`: **`zwlr_virtual_pointer_v1`**, a mouse that does not exist. Every
event it sends goes through the compositor's ordinary input handling — focus,
surface-local coordinates, the implicit grab — before coming back out to
whichever surface is under it, so what is measured is the whole path rather
than a shortcut into it. Four tests: that a press arrives with surface-local
coordinates *and* the desktop coordinates this client derives itself, that a
drag is motion between a press and a release, that a press somewhere else is
not delivered at all, and that a pointer leaving mid-drag keeps being
delivered — which is Wayland's implicit grab, a claim in `grab_pointer`'s own
documentation that nothing had checked.

**Two things had to exist first, and both are worth naming.**

A Wayland surface with **no buffer is not mapped**, and an unmapped surface is
not under the pointer no matter where the pointer is. A widget's pixels come
from wgpu, so the shipped desktop never needs shared memory — which left a test
with nothing on screen to click. `WaylandWindow::present_blank` attaches
4 × w × h bytes of zeroes, and lives behind a `test-support` feature that only
a `[dev-dependencies]` line turns on, exactly as `uniwidged_runtime::test_server`
does. Nothing in a release build can reach it.

And **a headless seat has no pointer capability at all** — `wl_seat.capabilities`
is literally zero — until a virtual one is created. A client binds its
`wl_pointer` from that event, so a widget that opened its surface while the
capability was still absent never took a pointer and every event afterwards
went to a compositor with nowhere to deliver it. The first version of the file
did exactly that and failed with an empty list and no other clue. The virtual
pointer is now flushed into existence before any surface is opened.

The compositor harness moved to `tests/wayland_harness/` on the way, because
two suites needed it and two copies of a compositor harness is two harnesses
drifting apart — the socket-length rule sway segfaults over being precisely the
kind of detail that gets fixed in one of them.

## Independent of its parents — **done, 2026-08-28**

Not a milestone, and it should have been true from the start. uniwidged
declared four path dependencies on a sibling checkout of Ferrum UI, of which
exactly one — `ferrum-layout` — was used by any crate here. The other three
were in the workspace manifest and in nothing else.

That checkout was a private repository, so `git clone && cargo test` did not
work for anybody but its author, and CI needed a token to fetch it. A framework
whose SDK cannot be built by the people it is *for* is not a framework yet.

So the box model was adopted: taffy behind a facade at
`uniwidged-compose::layout::boxes`, trimmed from ferrum-layout's public API to
the box model a uniwidged view can actually describe — no grid containers, no
scroll containers, no margins, and four space-distribution modes rather than
six, because `uniwidged_view::Justify` has four. Six tests came with it, three
of which are about the traps: a percentage of an `Auto` parent is zero, an
unmeasured leaf must never reach the measurement closure, and a box that may
not shrink keeps its width in a row that overflows — which is what a chip
depends on, and what once rendered "live" as "li…".

`NOTICE.md` records what came from where. Every dependency is now on crates.io.

## M8 · Shipping — **done, 2026-11-09** (Windows and macOS packaging moves with M5 and M6)

* Packaging: a `.deb` and an AUR package are **done** — see below; MSI/MSIX and
  a signed and notarised `.app` wait on the platforms they package, which are
  M5 and M6.
* Running without a terminal: a log, `uniwidged status`, and shell completions
  — **done**, see below. And the vocabulary's one chart node, `sparkline`,
  with `metrics` answering for memory as well — ABI 7. And a picture behind
  the `image` node that had never had one — ABI 8.
* A release: a version that is not `0.0.1`, a changelog, and a workflow that
  builds the `.deb` for a tag and attaches it — **done**, see
  [Towards 1.0.0](#towards-100).
* A widget registry format — **done**, see below. Signature verification on
  install was already done; what was missing was somewhere to find widgets, and
  now is not.
* The authoring guide, an API reference, and four reference widgets, each
  exercising a different capability.

**Exit:** somebody who has never seen the repository writes and installs a
widget from the guide alone.

### The authoring path — **done, 2026-08-28**

The exit criterion cannot be asserted, but its mechanical half can, and now is.

* **`uniwidged new`** scaffolds a widget that already builds, draws, responds
  to a click and has its own tests. Not a stub: the gap between "I read the
  guide" and "something appeared" is where frameworks lose people, and what it
  generates is something to *edit* rather than something to write from nothing.
  It validates the name through the runtime's own parser, so a name the
  scaffold accepts is one the host will run.
* **[`docs/writing-a-widget.md`](https://josunlp.github.io/uniwidged/guide/writing-a-widget)** — the guide. Nine
  sections, including the one most guides omit: what you *cannot* do, and why.
* **Four reference widgets**, one per served capability —
  [`clock`](https://github.com/JosunLP/uniwidged/tree/main/widgets/clock) (`locale`), [`system-load`](https://github.com/JosunLP/uniwidged/tree/main/widgets/system-load)
  (`metrics`), [`counter`](https://github.com/JosunLP/uniwidged/tree/main/widgets/counter) (`fs`) and
  [`uptime`](https://github.com/JosunLP/uniwidged/tree/main/widgets/uptime) (`command`). Each declares only what it uses, and a
  test asserts that list rather than deriving it: a worked example that
  over-declares teaches every reader to over-declare.

**Measured.** `crates/uniwidged-cli/tests/scaffold.rs` runs `uniwidged new`,
runs the generated crate's own tests, builds it for `wasm32-wasip2`,
instantiates it in the real runtime under a deny-all policy and clicks its
button. The guide's first four commands are checked on every change rather than
believed — and because the chain runs end to end, a change to the SDK, the
manifest format, the ABI or the component world surfaces here as the generated
project failing to build.

`crates/uniwidged-host/tests/reference_widgets.rs` does the same for the three
examples, installing each under **exactly** the capabilities its own manifest
declares. The counter's test kills the whole desktop and builds a new one over
the same directory, which is the demonstration that a widget's state is derived
from its data — the assumption the entire supervisor design rests on.

Writing the reference widgets found an API bug worth naming: `Widget::start`
took no `Cx`, so a widget could not load what it had saved in the one method
whose whole purpose is loading what it saved. Every widget with anything to
restore was going to end up doing it lazily in `view` behind a `loaded` flag.
It takes one now. That is what reference widgets are for.

### The API reference — **done, 2026-08-28**

[`docs/api-reference.md`](https://josunlp.github.io/uniwidged/reference/api) — every function, type, limit
and refusal on one page, with the ABI history at the bottom. The third of M8's
documentation bullets, and the one a guide cannot replace: `writing-a-widget.md`
is an argument with worked examples, and somebody halfway through a widget
wants a table.

Writing it found a claim nobody had checked. `.link(url)` marks a node as a
link, an assistive technology is told so — and **nothing opens it**. That is now
said in the reference rather than implied away, and it stays unbuilt on purpose:
handing a widget a way to make the host launch an arbitrary URL is a
capability, with `file://` and `x-scheme-handler` behind it, and it has not been
designed.

**Still to do at the time:** packaging and the registry format. Both are below.

### Somewhere to find widgets — **done, 2026-08-28**

Every previous milestone answered *may this widget do that* and *who wrote it*.
Neither answered the question before both, and the honest answer until now was
"somebody sends you a `.wasm` file", which works exactly as well as it sounds.

A registry is **one signed TOML document on a static host**. No accounts, no
API, no server to run. That is a deliberate ceiling: this project cannot
honestly operate infrastructure, and a distribution mechanism depending on
infrastructure nobody maintains stops working the year after it ships. Anything
larger is somebody else's layer on top of a public file whose format is a page
long — [`docs/registries.md`](https://josunlp.github.io/uniwidged/guide/registries).

**The interesting decision is that there are two signatures, by two people.**
The index is signed by whoever publishes the *list*, and that key is pinned
when the registry is added — `uniwidged registry add <url> --key <hex>`. A
widget is signed by whoever *wrote* it, and that key is in `trusted-keys`,
where it always was. They are different claims and are checked separately, with
domain-separated digests so a signature over one can never be replayed as a
signature over the other. The consequence is the property worth having: **a
registry cannot install anything.** It offers; the machine's own trusted keys
still decide. `a_widget_is_found_and_installed_from_a_registry` runs `add`
against a registry it trusts and watches it refuse, because the author is not
yet trusted.

**What a listing shows has to be what the runtime enforces.** An entry carries
the capabilities a widget declares, because somebody choosing what to install
reads that before anything is downloaded. So the manifest that arrives must
agree with the entry that offered it — same name, same version, same
capabilities — or the install is refused *even when every signature is valid
and every hash matches*. Without that check, a listing is a place to put one
sentence in front of somebody and run another.

**And the digest is the one a signature covers**, over the component and the
manifest together, rather than a hash of the `.wasm`. Same reason as M8's
signing work: a digest over the component alone lets somebody keep the bytes
and widen the manifest, and the manifest is the whole of what a user approves.

`uniwidged update` is the one command that installs something nobody looked at,
so it is the one command that may not widen a permission. An upgrade asking for
anything the installed grant does not already cover is reported and **not**
applied; taking it means running `add`, where the whole list is printed. What
*is* carried across is where the widget sat, what it was configured with and
whether it was switched off — a new version of a widget is not a new widget,
and that fixed an older bug on the way: `install` over an existing widget used
to move it back to the corner and forget its settings.

**Two things fell out of writing it that are worth naming.**

The first is that *a registry being down and a registry serving a forged list
are different facts*. The first version had one `Err(String)` and a cache
fallback, and the end-to-end test caught it immediately: an index edited after
it was signed produced a grey warning line and the command carried on with the
cached copy. `FetchError` now separates transport from refusal, and only the
first falls back.

The second is that **the TLS test server was two hundred lines locked inside
one module's tests**. `uniwidged add` needed a real index, a real component and
two real signatures over real HTTPS, and a second copy of that server would
have been a second server drifting from the first. It moved to
`uniwidged_runtime::test_server` behind a `test-support` feature that only a
`[dev-dependencies]` line turns on, so nothing in a release build sees it — and
the CLI's own tests now drive the real binary against a real server, with the
authority handed to it through `UNIWIDGED_REGISTRY_CA`, which is also how a
registry inside an organisation is configured.

Versions had to become orderable for any of this: `1.10.0` is newer than
`1.9.0` and a string comparison says otherwise. `uniwidged_runtime::version` is
semver's ordering, including the rule that `1.0.0-rc.1` precedes `1.0.0` — an
update check that got *that* backwards would offer a release candidate to
somebody already running the release. A manifest's version is now checked when
it is parsed rather than carried as an unexamined string.

### Packaging — **partly done, 2026-08-28**

`packaging/deb/build.sh` builds a `.deb` with `dpkg-deb` and nothing else: no
`cargo-deb`, no `dh`, no container. The package is a binary, a manual page and
a desktop entry, and a control file that can be checked by reading — which
matters more here than the convenience of a tool that writes it in one line.
The shared-library dependencies are asked of the binary rather than listed by
hand.

**CI builds it and then looks inside it**: the binary, the manual page and the
desktop entry are all there, `dpkg-deb` accepts the control file, the version
in the package is the version in `Cargo.toml`, and the binary that comes out of
it runs and agrees about its own version. It is deliberately *not* an
installation test — installing as root and starting a session under it is a
virtual machine, and a job pretending otherwise would be a green tick meaning
nothing.

`packaging/PKGBUILD` is written against `makepkg`'s documented behaviour and is
**not** built here, and says so rather than implying it was.

**Still to do:** MSI/MSIX and a signed, notarised `.app`. Both need the
platform they package for, and packaging a binary that prints "this build has
no shell for the current platform" would be packaging nothing. They come with
M5 and M6, in the same commit as something to run. Flatpak is **not planned**:
a widget host draws below every window, reads the session's fonts, watches
RandR and verifies a signature before running anybody's code, and a sandbox
that has to be opened up for each of those is a sandbox holding nothing. The
interesting sandbox in this project is the one inside the process, around each
widget.

### Saying what it is doing, a chart, and the shell — **done, 2026-08-31**

Three things a program that runs without a terminal needs and this one did not
have, and one thing the view vocabulary was missing.

#### A log, and `status`

`uniwidged run` prints sixty different lines somebody needs at the moment they
are wondering why their desktop looks wrong, and the normal way a widget host
runs is started with the session — no terminal, standard output to the journal
if the user is lucky. So the desktop keeps a log beside its configuration:
append-only, timestamped to the second, `!` on the lines that went wrong, a
megabyte and one rotation so the disk holds at most two. `uniwidged log` reads
the tail, and reaches into the rotated file when the current one is shorter
than asked for, because the interesting line is usually the one just before a
rotation. `-f` follows. Nothing in it can fail loudly: a log that stops a
desktop is worse than no log.

`tracing` had been in the workspace manifest since the first commit and was
used by nothing. It is gone; what replaced it is a hundred lines, which is the
size of the problem.

`uniwidged status` answers the question `list` reads a file to avoid: what is
on the screen. The desktop writes a state file whenever the answer changes — a
widget started, stopped, moved, given up on, the monitors re-resolved — and
never on a plain frame, so an idle desktop writes nothing. It is written to a
temporary file and renamed, like everything else here, because `status` is
most likely to be run at exactly the moment something is changing. And it is
**never believed on its own**: a desktop killed with `SIGKILL` leaves it behind
beside the pid file, so every reader checks the pid first, and a state file
naming a different desktop than the pid file is refused as stale rather than
read. `list` uses the same file to say `(on screen)`, `(given up on)` or `(not
running)` beside a widget the configuration enables.

Writing this found a bug worth its own paragraph. The start-up loop had a `?`
on each widget: a deleted `.wasm`, a manifest somebody had broken while
editing it, or a component built for the wrong target emptied the *whole*
screen and printed one line about one widget — at the moment the user could
least see what else they had. Each failure is now named and the rest start;
only a desktop where nothing started is an error. Which exposed a second bug
the first had been hiding: placement indexed `enabled` and `running` with the
same number, so a widget that did not start would have put every later widget
where its neighbour belongs. Placement is by name now.

Measured in `crates/uniwidged-cli/tests/live.rs`, against the real binary on
a real X server: `status` names the widget and the shell while it runs, loses
the widget when it is disabled, and refuses the stale files after the desktop
is killed. The rotation and the timestamp format are unit tests against a
path each test owns — the first draft set `UNIWIDGED_CONFIG` on the process,
which is a variable every other test in the binary reads, and would have been
a flake with a comment claiming otherwise.

#### `sparkline`, and memory — ABI 7

"A `Canvas` node for charts is a *maybe*, and would still be host-drawn from a
declarative series." It is a yes, and it is still that. `sparkline(values)`
carries a series of `Ratio`s across the boundary — oldest first, each clamped,
at most 256 — and the host draws a line, a faint area under it and a marker on
the latest point, in the theme's stroke and the node's accent. The widget names
no pixel. That is the whole of the vocabulary's argument applied to the one
node that could have been an escape hatch, and the reason it goes no further.

The cap is the view validator's rather than the SDK's, and refused rather than
truncated there: a Rust widget cannot send more, because `build::sparkline`
keeps the recent end, but a hand-built arena from a widget in another language
can, and 100 000 points is a request for the host to draw 100 000 segments
inside a frame budget. The SDK's truncation is a courtesy to authors; the
validator's refusal is the security boundary. Four golden images, one of them
mirrored — time runs right to left in a right-to-left layout, and that is a
claim a picture has to make.

`memory-load` joins `cpu-load` under the one `metrics` grant, from
`MemAvailable` rather than `MemFree`, because the second counts the page cache
and reads 95 % on any machine that has been up for an hour; a kernel without
`MemAvailable` (before 3.14) is answered with a refusal rather than an
estimate. The conformance widget tries it as its own row, so a host that serves
one counter and forgets the other fails a test; and it draws a sparkline, so a
host that cannot decode the variant fails at the boundary with the ABI number
in the sentence rather than in whichever widget first uses it.
`widgets/system-load` keeps a minute of readings and shows both.

#### The shell

`uniwidged completions bash|zsh|fish` writes a completion script from a table
in the binary, and widget names complete from `uniwidged list --names` — one
name per line, the one output of that command that is a format rather than a
sentence. The table is checked against the commands `main` dispatches, so a
command added without a row fails a test; the bash script is parsed by bash
in the test, and the other two where their shells are installed. The `.deb`
asks the binary for all three and installs them where each shell's Debian
packaging looks, and CI checks they are in the package.

### A picture — ABI 8 — **done, 2026-08-31**

`content.image(u32)` had been in the interface since the first version, and
there was no way to obtain a `u32` that meant anything. `register-image` is
the other half: a PNG crosses the boundary once, the host decodes it — after
checking the header, so a file claiming 100 000 pixels a side is refused by
its dimensions rather than by an allocation that succeeds or fails depending on
the machine — and keeps the pixels behind a handle. The renderer is handed the
same `Arc` every frame, so a renderer that caches uploads by identity uploads
an icon once rather than sixty times a second; the host's cache is brought up
to date by pointer comparison before each compose.

No capability, and the argument is in `crates/uniwidged-runtime/src/images.rs`:
the bytes came from the widget and go to the widget's own surface, so there is
nothing for a user to consent to, and what is bounded instead is the host's
memory — a megabyte, 1024 pixels a side, sixteen at once. PNG only, because one
format is one decoder, and a decoder is a parser running on bytes a widget
chose. A handle belongs to the instance that made it: a restarted widget
registers again, which is the same rule as everything else about a restart.

Two golden images, one mirrored — a right-to-left layout moves the picture to
the leading edge and does not flip it, for the reason glyphs are not flipped.
The conformance widget registers its four pixels in `start`, offers the host
bytes that are not a PNG, and the test reads the decoded pixels back off the
instance in the order the file holds them.

### The rest of the machine — ABI 9 — **done, 2026-11-07**

`metrics` answered two questions, and a person glancing at a desktop asks five:
how busy, how full, how much memory, how much battery, how much network.
`battery`, `disk-load` and `network-throughput` join `cpu-load` and
`memory-load` under the one grant, because the sentence a user approves —
"this widget reads how the machine is doing" — has not changed. Each is a small
parser of a kernel file in `crates/uniwidged-runtime/src/machine.rs`, and the
parsers take a directory or a string so the tests can hand them a laptop, a
desktop and a server without owning any of the three.

The decisions worth writing down:

* **No battery is `ok(none)`.** The rule `read-file` and `get-secret` learned in
  ABI 2 and ABI 5, applied before anybody had to learn it a third time: a
  desktop's normal case is not a failure, and a widget made to tell "this
  machine runs on mains" from "this host is broken" by reading an error string
  would get it wrong.
* **Three power states, not five.** "Full", "Not charging" and the "Unknown"
  some firmware reports at a charge limit are one sentence to somebody looking
  at a widget: plugged in, and not gaining.
* **No estimate the kernel did not make.** `minutes-left` is `none` where there
  is no rate to divide by, and past a day — the second after a cable moves,
  some drivers report a few microwatts, and "4 000 hours left" is arithmetic,
  not an estimate.
* **Weighted, not averaged.** Two batteries are combined by capacity, and a
  peripheral's (`scope = Device`: a mouse, a headset) is not the machine's.
* **The disk the way `df` counts it.** The administrator's reserve is used,
  because an ordinary user cannot write to it; `used / total` reads 95 % at
  the moment writes start failing.
* **The network sampled like the CPU.** A rate needs two samples, and the
  second is taken at the next call rather than by sleeping, with its own
  window so a widget reading the CPU does not shorten the network's. Loopback
  is the machine talking to itself and is not counted; a total that went down
  is an interface that went away, and a window of zero rather than an
  underflow.

Two reference widgets for the two shapes the others did not show. `battery` is
what to draw when there is nothing to measure; `network` is a rate in a chart
that takes fractions, scaled against the busiest moment with a floor so that
keep-alives are not a mountain range, and with the scale written under it. Both
are installed under exactly their manifests' grants in
`crates/uniwidged-host/tests/reference_widgets.rs`, and the conformance widget
has a row for each new function, so a host that serves one and forgets another
fails a test rather than a sentence here.

And `uniwidged doctor` now says what each counter answers on the machine it
runs on — one line of the 1.0 criterion below, "`doctor` names the ones this
machine will produce", for the capability with the most of them.

### Reference widgets, one per served capability

| | |
|---|---|
| [`clock`](https://github.com/JosunLP/uniwidged/tree/main/widgets/clock) | `locale` |
| [`system-load`](https://github.com/JosunLP/uniwidged/tree/main/widgets/system-load) | `metrics` |
| [`battery`](https://github.com/JosunLP/uniwidged/tree/main/widgets/battery) | `metrics`, with nothing to measure |
| [`network`](https://github.com/JosunLP/uniwidged/tree/main/widgets/network) | `metrics`, as a rate |
| [`counter`](https://github.com/JosunLP/uniwidged/tree/main/widgets/counter) | `fs` |
| [`uptime`](https://github.com/JosunLP/uniwidged/tree/main/widgets/uptime) | `command` |
| [`vault`](https://github.com/JosunLP/uniwidged/tree/main/widgets/vault) | `secrets` |

`net` has none, and that is the gap in the set: a reference widget for it needs
a service to talk to that will still be there in a year, and pointing the
project's own worked example at somebody else's free API is a broken example
waiting to happen. `widgets/conformance` exercises the gate; `net`'s own suite
exercises the transport against a server it starts itself.

---

## Towards 1.0.0

**0.1.0** was the first version with a number that meant something: the Linux
desktop worked on both window systems, every capability was served, widgets
could be found, checked, installed, configured, moved and watched, and the
whole of it was measured in CI.

The criteria written here for 1.0 were four. This is what became of each, and
why the number was taken anyway — so that it was decided rather than reached
by counting.

* **The ABI stops moving — done.** ABI 9 is frozen for 1.x. The check at
  instantiation became a range, `ABI_OLDEST..=ABI_VERSION` with the floor at 9,
  so an interface that grows by adding host functions keeps running every
  widget built against 1.0. What may change in 1.x and what waits for 2.0 is
  written down in
  [ABI & stability](https://josunlp.github.io/uniwidged/reference/abi), and a
  refusal now says whose move it is.
* **What is not there is said — done.** Every `Unavailable` is documented
  beside the function that answers it, `uniwidged doctor` names the ones this
  machine will produce, and the documentation site has a troubleshooting page
  in the words the log uses.
* **M5 and M6 — moved to 1.x.** The argument for waiting was that a framework
  calling itself cross-platform is not 1.0 on one platform. The argument
  against is stronger: the thing a 1.0 number promises a widget author is that
  their widget keeps working, and Windows and macOS are **additions behind the
  same interface** — nothing about them can change what a widget sees. Holding
  the stability promise back until two more shells exist would have kept every
  author on an unstable number for no benefit to them. So 1.0 is Linux, says
  so, and each further platform is a minor release.
* **A year of somebody else's widget — replaced by its mechanical half.** A
  year cannot be shortened, and it was never a property of the code. What the
  code can promise is that the guide's commands work as written for somebody
  who has never seen the repository, and getting there found a real bug: the
  guide's first `uniwidged install` was refused, because `cargo build` puts no
  manifest beside the component. `install` now finds the crate's own
  `widget.toml`, `uniwidged new` outside a checkout depends on the SDK by git
  at its release tag, and both are tested.

### What 1.0.0 added

* The stability promise above, and the ABI range that implements it.
* A documentation site — VitePress, in `docs/`, deployed to GitHub Pages — with
  a guide, the API and CLI references, the security model and this file.
* A logo and a brand, in `assets/`.
* `uniwidged help <command>` and `--help` on every command.
* `now_seconds()` in the SDK prelude.
* `CONTRIBUTING.md`, `SECURITY.md`, issue forms and a pull request template;
  release notes taken from `CHANGELOG.md`.

## After 1.0.0

Minor versions, each additive:

* **1.x · Windows (M5).** A DirectComposition shell kept at `HWND_BOTTOM`,
  DPAPI behind `secrets`, MSI/MSIX packaging, and the measured tests — see M5
  above.
* **1.x · macOS (M6).** A desktop-level `NSWindow` with a `CAMetalLayer`, the
  Keychain behind `secrets`, a signed and notarised `.app` — see M6 above.
* **1.x · The SDK on crates.io**, so a widget can depend on
  `uniwidged-guest = "1"` rather than a git tag.
* **1.x · A second SDK.** The interface is a WIT file; a non-Rust SDK is the
  test that it is a contract rather than a Rust API with extra steps.

Patches for anything that is a fix. The `.deb` and the AUR package follow the
workspace version, and nothing else has one. **2.0** only if the interface has
to break — and the ABI page says what that would take.

## Not planned

Named so that nobody has to ask twice:

* **A general application framework.** That is Ferrum UI, which this took its
  rendering techniques from and which remains the place for one.
* **An HTML/CSS/JS runtime.** The point of §6 of `ARCHITECTURE.md` is that
  widgets do not bring their own renderer.
* **Widgets that draw their own pixels.** Same reason. `sparkline` is as far
  as that goes: a declarative series, host-drawn, and the argument for stopping
  there is above.
* **A panel or status-bar replacement.** The shell layer would serve one; it is
  somebody else's project.
