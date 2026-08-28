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

**Exit:** the widget stays below a maximised window, never steals focus, has no
Alt-Tab entry, and shows the wallpaper through its glass.

## M6 · macOS

* Borderless `NSWindow` subclass with `canBecomeKeyWindow → NO`, level
  `kCGDesktopIconWindowLevel + 1`, `.canJoinAllSpaces | .stationary |
  .ignoresCycle`, `.accessory` activation policy.
* Layer-backed `NSView` with a non-opaque `CAMetalLayer`; wgpu attached via
  `SurfaceTargetUnsafe::CoreAnimationLayer`.
* `NSVisualEffectView` behind it for the backdrop.

**Exit:** as M5, plus: the widget follows the user across Spaces and does not
slide away under Mission Control.

## M7 · The host and the CLI — **partly done, 2026-08-27**

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

**A fourth reference widget.** [`widgets/uptime`](widgets/uptime) is the
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

* **The keychain namespace, and the network.** `secrets` and `net` still answer
  `unavailable`. The conformance suite asserts that, so implementing one and
  forgetting the list fails a test. `net` is now a *transport* rather than a
  design: the token machinery, the delivery and the wake are all built and
  tested, and a client slots into `Tasks::start`.
* **A disk quota.** The cap is per file, not per widget: a widget granted `fs`
  can still fill a disk one file at a time.
* **Two monitors, actually tested.** The placement logic is tested against a
  docked-laptop layout, and the enumeration against a nested server — which has
  one screen. Nobody has yet watched a widget move between two real monitors.
* **Autostart.**
* **Pointer input on Wayland**, which is wired but unmeasured — headless sway
  has no input device. The shared interaction model above it is covered by the
  X11 pointer tests; what is unverified is the Wayland plumbing into it.

The milestone's original exit criterion — eight widgets across two monitors
surviving a restart — is met except for the two monitors, which this machine
does not have. Everything that criterion stood in for holds: a desktop full of
real sandboxed widgets runs, draws, responds, can be moved and resized and
pinned, remembers where it was put, and costs nothing at rest.

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

## M8 · Shipping — **partly done, 2026-08-28**

* Packaging: MSI/MSIX, a signed and notarised `.app`, Flatpak, an AUR package,
  a `.deb`.
* A widget registry format, and signature verification on install.
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
* **[`docs/writing-a-widget.md`](docs/writing-a-widget.md)** — the guide. Nine
  sections, including the one most guides omit: what you *cannot* do, and why.
* **Four reference widgets**, one per served capability —
  [`clock`](widgets/clock) (`locale`), [`system-load`](widgets/system-load)
  (`metrics`), [`counter`](widgets/counter) (`fs`) and
  [`uptime`](widgets/uptime) (`command`). Each declares only what it uses, and a
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

**Still to do:** packaging, the registry format and signature verification, an
API reference, and the weather widget.

---

## Not planned

Named so that nobody has to ask twice:

* **A general application framework.** That is Ferrum UI, which this took its
  rendering techniques from and which remains the place for one.
* **An HTML/CSS/JS runtime.** The point of §6 of `ARCHITECTURE.md` is that
  widgets do not bring their own renderer.
* **Widgets that draw their own pixels.** Same reason. A `Canvas` node for
  charts is a *maybe*, and would still be host-drawn from a declarative series.
* **A panel or status-bar replacement.** The shell layer would serve one; it is
  somebody else's project.
