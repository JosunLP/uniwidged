# uniwidged — Architecture

uniwidged is a **desktop widget framework**: a host process that renders many
small, always-on panels onto the desktop — below every ordinary window, above
the wallpaper — and a sandboxed SDK third parties write those panels against.

It is the union of two existing projects. [Ephemeris](https://github.com/JosunLP/Ephemeris)
worked out how a window becomes a *desktop widget* on four platforms and is the
source of `uniwidged-shell`. [Ferrum UI](https://github.com/j-pfalzgraf/ferrumui)
is a GPU-rendered UI stack on Vello/wgpu/Parley/taffy/AccessKit and is where
the techniques above the surface come from. Both are **adopted rather than
depended on** — this workspace builds from crates.io alone, with no sibling
checkout; see `NOTICE.md`. What is genuinely new is the seam between them — a
raw desktop-layer surface handed to wgpu instead of to Cairo — and the WASM
boundary that lets somebody else write a widget.

---

## 1. What this is, and what it is not

**Is.** A single host process owning one GPU device and N desktop surfaces. A
widget is a WASM component that returns a description of what it wants shown;
the host lays it out, shapes its text, paints it, hit-tests it and exposes it
to screen readers.

**Is not.** Not a general application framework — that is Ferrum UI, which
uniwidged took its rendering techniques from and which remains the place for
one. Not a wallpaper engine or a shader playground. Not an
HTML/CSS/JS runtime. Not a panel or status-bar replacement, though the shell
layer would serve one.

## 2. Design principles

1. **The host owns the pixels.** A widget never draws. It emits a semantic view
   tree; the host decides what that looks like. Twenty widgets that could each
   paint would look like twenty different toolkits, and the one thing a desktop
   full of widgets must have is a single visual voice.
2. **Nothing runs when nothing changes.** There is no idle loop and no idle
   frame. The scheduler sleeps until the earliest thing any widget asked to be
   woken for. This is the whole performance story; everything else is detail.
3. **Platform code at the edges only.** `uniwidged-shell` is the only crate
   that knows an OS exists. Everything else compiles and behaves identically,
   and is testable without a display.
4. **A widget cannot hurt the desktop.** No ambient authority, a declared
   capability list, a memory cap, and a deadline on every call into guest code.
   A widget that hangs is suspended showing its last frame; the desktop does
   not notice.
5. **Quality is verified, not asserted.** Every visual claim in §7 has a
   golden-image test behind it, run headlessly in CI on a software rasterizer.

## 3. The crate stack

```
                        ┌───────────────────────────────┐
   widget author ─────► │ uniwidged-guest   (WASM SDK)  │  → wasm32-wasip2
                        └───────────────┬───────────────┘
                                        │  WIT component interface
 ═══════════════════════════════════════╪═══════════════════ sandbox boundary
                                        │
                        ┌───────────────▼───────────────┐
                        │ uniwidged-runtime  (Wasmtime) │  instantiate · capabilities
                        │                               │  memory cap · epoch deadline
                        └───────────────┬───────────────┘
                                        │  View  (pure data, no OS, no GPU)
                        ┌───────────────▼───────────────┐
                        │ uniwidged-view                │  nodes + style tokens
                        └───────────────┬───────────────┘
                                        │
  ┌─────────────────┐   ┌───────────────▼───────────────┐
  │ uniwidged-host  │◄─►│ uniwidged-compose             │  taffy layout · Parley text
  │  scheduler      │   │   → Scene · hit rects · a11y  │  → vello::Scene
  │  config · IPC   │   └───────────────┬───────────────┘
  │  capability     │                   │  vello::Scene
  │  policy         │   ┌───────────────▼───────────────┐
  └────────┬────────┘   │ uniwidged-render   (wgpu)     │  1 Device · N surfaces
           │            │                               │  1 submit per frame
           │            └───────────────┬───────────────┘
           │                            │  raw-window-handle
           │            ┌───────────────▼───────────────┐
           └───────────►│ uniwidged-shell               │  desktop-layer surfaces
                        │  wayland │ x11 │ win32 │ appkit│  + input + configure + DPI
                        └───────────────┬───────────────┘
                                        │
                        ┌───────────────▼───────────────┐
                        │ uniwidged-core                │  geometry · color · ids
                        │                               │  events · wake
                        └───────────────────────────────┘
```

| Crate | Owns | Depends on an OS? | Needs a GPU? |
|---|---|---|---|
| `uniwidged-core` | geometry, colour, ids, input events, the wake model | no | no |
| `uniwidged-view` | the view tree and style tokens that cross the sandbox | no | no |
| `uniwidged-compose` | view → layout → text → `vello::Scene`, hit rects, a11y nodes | no | no |
| `uniwidged-render` | the wgpu device, per-surface renderers, readback presentation | no | yes |
| `uniwidged-shell` | desktop-layer surfaces, input, configure, scale, menus | **yes** | no |
| `uniwidged-runtime` | Wasmtime, capability enforcement, resource limits | no | no |
| `uniwidged-guest` | the SDK widget authors compile against | no | no |
| `uniwidged-host` | the daemon: scheduler, config, policy, lifecycle | no | no |
| `uniwidged-cli` | `uniwidged run · new · install · doctor · snapshot` | no | no |

Two of these compile to WASM (`uniwidged-core`, `uniwidged-view`) because the
guest SDK shares their types with the host. That is deliberate: the view tree a
widget builds and the view tree the host lays out are the *same Rust type*, not
two definitions that must be kept in sync.

## 4. Data flow of one frame

```
 scheduler wakes            (a timer is due · input arrived · a fetch finished)
      │
      ▼
 for each widget marked dirty:
      runtime.view(id)  ──► guest `view()` runs under a deadline
                       ◄──  View tree
      │
      ▼
 diff View against the last one
      │  unchanged → the surface stays clean, no frame is built at all
      ▼
 compose:  View ──► taffy layout ──► Parley shape/measure ──► vello::Scene
                                 └──► absolute hit rects (for input)
                                 └──► AccessKit nodes  (for screen readers)
      │
      ▼
 render:   per dirty surface: Vello compute pass → intermediate texture
           then ONE CommandEncoder blits every one of them onto its
           swapchain image, and submits once
             ← clean surfaces are not encoded, not submitted, not presented
      │
      ▼
 present each dirty surface
      │
      ▼
 next_wake = min over all widgets     ──► sleep
```

Input runs the other way: `uniwidged-shell` turns a native event into a
`uniwidged_core::PointerEvent`/`KeyEvent`, the host hit-tests it against the
recorded rects of that surface, and delivers a semantic event (`clicked(node)`,
`hovered(node)`) into the guest — never raw coordinates it would have to
interpret itself.

## 5. The wake model

This is where the performance requirement actually lives. Ephemeris costs
0.03 s of CPU per 40 s of idle, and it does so by having no idle loop: three
timers, each alive only while something needs it. uniwidged generalises that
to N widgets.

```rust
pub enum Wake {
    /// Only external events — input, a capability callback, a config change.
    Never,
    /// A wall-clock instant. A clock asks for the top of the next minute.
    At(SystemTime),
    /// A delay from now. A poller asks for its interval.
    After(Duration),
    /// The display's refresh rate, while something is genuinely moving.
    Animating,
}
```

Every call into a widget returns one. The scheduler sleeps until the earliest
across all of them, and the host submits **no frame** in between.

The rules that keep this honest:

* **No widget may poll.** A capability that waits — an HTTP fetch, a file
  watch, a D-Bus signal — runs on the host's async runtime, off the wake path,
  and wakes its widget when it has something. A widget never spins to find out.
* **`Animating` is auto-clearing.** It is granted for the duration of an
  animation the host is driving. A widget that returns `Animating` across many
  frames with a byte-identical view tree is logged, and then throttled.
* **The clock aligns.** Widgets asking for "the next minute" are coalesced onto
  the same wake, so eight clocks cost one wake-up, not eight.

**Budget.** With eight widgets installed and none animating, the target is
under 0.1 % CPU averaged over a minute, zero frames submitted, and the GPU at
its idle clock. Regressions are caught by a soak test, not by hoping.

**Blocking on nothing means blocking on everything.** The loop's block is a
single `poll`, and what it has to cover is every descriptor a widget could be
woken through. On Linux that is more than one: a window carries its own
connection — `XCBConnection::connect` per X11 window, a `wl_display` per
Wayland one — because a window is opened by whoever wants one and neither
protocol has a session object above them. Waiting on the first window's
descriptor is therefore waiting on the *first widget*, and with `next_timeout`
answering `None` — the property this whole section is about — a click on the
second sat unread until something else happened to wake the process. So the
loop polls every connection and the wake pipe together, and asks each window
whether it already has something queued *before* it decides to sleep at all:
an event that arrived between the last drain and this call is in a queue, not
on a socket, and no amount of polling will report it.

## 6. The sandbox boundary

### What crosses it

The guest returns a **view tree**: nodes with a role, a style token, text or an
image handle, and children. It is pure data — a few hundred bytes to a few
kilobytes — and it is the *only* thing that comes back.

It crosses **flat**, as an arena: a list of nodes, each naming its children by
index, with node 0 as the root. WIT has no recursive types, so a record cannot
contain a list of itself and there was no choice about this — but it turns out
to be the better shape anyway. One allocation per side instead of one per node,
and the host can check the whole structure before touching any of it.

The check matters, because the arena arrives from a sandbox. The rule that does
the work is that **a child's index must be greater than its parent's**: an
arena must be topologically ordered, which makes a cycle impossible to *express*
rather than merely detected. There is no visited set to get wrong and no depth
at which the check gives up — and a recursive decoder would have discovered a
cycle by running out of stack, which on a desktop process means the widget takes
the desktop with it. Two parents, an index out of range, an unattached node, too
many nodes and an over-long string are refused alongside it. See
`uniwidged_view::wire`.

The guest never sees a pixel, a font, a GPU, a window, or a coordinate it did
not ask for. Layout, text shaping, painting, hit-testing and the accessibility
tree are all the host's.

Four reasons, in order of weight:

1. **One visual voice.** Style is named, not specified: a widget asks for
   `Style::Title`, not for 15 px semibold #E8E8EA. The theme resolves it. This
   is what makes a screen full of third-party widgets look like one product.
2. **Text is done once, well.** Shaping, bidi, font fallback and hinting are
   the hardest and heaviest part of a UI stack. Every widget shipping its own
   copy would be larger *and* worse.
3. **The ABI stays small.** A view tree is kilobytes. A pixel buffer would be
   megabytes per frame across a sandbox boundary — the thing that makes every
   web-technology widget framework cost what it costs.
4. **Accessibility comes free.** The host knows the semantic tree, so AccessKit
   nodes fall out of compose with no effort from the widget author.

### Capabilities

A widget declares what it needs; the user approves it at install; the runtime
enforces it. WASI Preview 2 has no ambient authority, so anything undeclared is
simply absent rather than denied.

| Capability | Grants |
|---|---|
| `timer` | returning a `Wake` other than `Never` |
| `net:<host>` | HTTPS to exactly those hosts, through the host's client |
| `fs:<subpath>` | read/write inside the widget's own data directory |
| `link:<host>` | opening `https://<host>` in the browser, when the *user* clicks |
| `secrets` | the keychain, namespaced to this widget |
| `locale` | the user's locale-formatted dates, times and numbers |
| `measure` | measuring a string in a named style, for its own truncation calls |
| `command:<name>` | running one declared external command |
| `metrics` | CPU, memory, battery, network counters |

`net` is host-scoped rather than a blanket switch because "this weather widget
talks to `api.open-meteo.com`" is a sentence a user can consent to, and
"this widget uses the network" is not.

**Host-scoping only means anything if redirects are re-checked.** A grant for
`api.example.com` becomes a grant for anywhere that host points the moment a
`302` is followed blindly — and the widget need not be complicit, because the
server decides. So `uniwidged-runtime::net` follows redirects itself rather
than letting the client library do it, and every hop goes back through the same
policy. The same function reads the host for the check and for the fetch: two
parsers disagreeing about what `https://granted.example@evil.test/` means is
the hole this closes.

The gate and what is behind it are separate, and a widget can tell them apart.
`Policy` decides whether a widget *may*; `Services` decides whether this host
*can*. An ungranted capability answers `not-granted`, and a granted one with
nothing behind it answers `unavailable` — which are different facts, and a
widget told the first when the second is true sends its author to fix a
manifest that is already right. Every capability in the table is served on
Linux as of 2026-08-28; the conformance suite asserts the unserved list from
the widget's side, so a capability that stops being served — or a new one that
is added and forgotten — fails a test rather than a sentence in a document.

### Links, which are a different question from `net`

`.link(url)` on a node makes it a link, and the host opens it when the user
clicks. A widget never launches anything — the host decides *whether*, and the
caller that owns windows does the launching, which is the same split
`ARCHITECTURE.md` §5 makes about surfaces.

It is a separate capability from `net` because they are separate sentences.
`net:api.example.com` is "this widget talks to a server"; `link:example.com` is
"this widget can put something on your screen that takes you to a web page".
A widget may want either without the other — one that reads a local sensor and
links to a forecast has no network grant at all — and a user may reasonably say
yes to one and no to the other. Neither implies the other, and the two matchers
are separate functions so that a wildcard cannot come to mean one thing across
both.

Two checks, at the moment of the click rather than when the view was composed:
the URL is read by **the same function `net` reads one with**, so anything that
is not `https` is refused before a host comparison happens at all; and then the
widget's own grant. A widget can draw whatever link it likes — it owns its
surface — and what is gated is the moment the browser would be launched, which
is the only moment that does anything. It is told about the click either way:
withholding the *news* would leave a widget unable to explain itself.

Refusing every scheme but `https` is the load-bearing half. `file://` opens a
file manager or an editor, and a registered `x-scheme-handler` opens whatever
somebody installed — so a system opener handed an arbitrary URL is a widget
running arbitrary programs by another name.

### The keychain

`secrets` was the last gate with nothing behind it, and it was right to be
last: it is the only capability where getting the storage wrong is worse than
not having it. A widget that cannot reach a keychain shows a sign-in button. A
widget whose token was written somewhere readable has already leaked it, and
nothing later undoes that.

So the shape is a seam, not an implementation. `SecretStore` is the trait;
`Keyring` is the per-widget handle cut out of it, carrying the namespace the
same way `TaskHandle` carries the widget's id — there is no argument through
which a host function could name somebody else's secrets.

Behind it today is one store: a file, encrypted whole with XChaCha20-Poly1305
under a key in a second file at mode `0600`. The honest description of what
that buys is the one `uniwidged doctor` prints and `install` shows at the
moment of consent:

* **It protects against** a backup, a synced dotfiles repository, a shared
  screen, another user on the machine, and anybody who ends up with the store
  without the key. Also against *editing*: the store is one AEAD message, so a
  file somebody changed fails to open rather than decrypting to something a
  widget then believes. And the store is one message rather than one per entry
  because the key *names* are as telling as the values — `github.token` says
  most of it.
* **It does not protect against** a program already running as the user.
  Nothing that keeps its key on the same disk can, and a framework that implied
  otherwise would have people store things here that they should not. A session
  keychain — the Secret Service, DPAPI, the macOS keychain — can, because the
  key is held by a daemon that a login unlocked and that can refuse. Those are
  backends behind the same trait.

Two decisions come from these calls happening inside a widget's `view`, on its
sixteen-millisecond deadline. A **read is memory**: the store is opened once
and every `get` after that is a lookup. A **write has no `fsync`**: the first
version had one, and an `fsync` waits on a device, so under a parallel test run
it interrupted widgets for the crime of storing a token — the same mistake
`Tasks::start` made by spawning a thread per request. The atomic rename is what
actually matters, and it is kept: a power cut can cost the most recent secret,
and cannot leave a keychain that will not open.

`measure` is worth a sentence of its own, because it is the one capability that
hands a widget something the host would otherwise keep. The handle behind it
shares the compositor's font context and follows its theme, so the width a
widget is told is the width the layout will use — a widget deciding its own
truncation against a different number is deciding against a guess, and a
measurer that copied the theme would keep answering plausibly after a light/dark
switch.

### Configuration

A widget declares what it can be configured with; the host validates and
stores; the widget reads. That division is the design, and it is the same one
as capabilities — declared, shown to the user, enforced by the host — applied
to a different question.

```toml
[[setting]]
key = "warn-at"
type = "number"
default = 0.6
min = 0.0
max = 1.0
```

It buys three things a string map does not. **A widget can never receive a
value it cannot handle**, so there is no branch for "the user typed `banana`
into a number" — the user was told at the moment they typed it. **The user can
be shown a form**: a toggle is a switch, a choice is a list, a bounded number
is a slider, and none of that is derivable from a widget that parses its own
strings. And **a stale key is visible**: the declared set is the authority, so
a configuration naming something the manifest no longer declares is a fact
somebody can be told about.

That last one is reported rather than fatal, and the asymmetry with an
over-grant is deliberate. An over-grant is a *security* fact — the file exceeds
what the user approved — and obliging it would hand a widget something nobody
agreed to, so it refuses. A stale setting is an *upgrade artefact*: a widget
dropped a knob in its new version, and refusing to start the desktop over that
would make every upgrade a support question.

Settings are **not a capability**. There is no argument through which to reach
another widget's, the values came from this widget's own manifest, and a user
who declined them would be declining the widget rather than a permission. `log`
is the other host function with the same shape and the same answer.

The handle is *shared* with the running instance rather than copied into it, so
a change reaches a widget that is already started. Reconfiguring by restarting
would throw away everything the widget had loaded in order to alter a number —
and `Event::Configured`, which has been in `wit/uniwidged.wit` since the first
version with nothing to produce it, is what tells it.

### Reading the configuration again

A desktop that had to be restarted to change a setting is a desktop where
nobody changes settings twice. `SIGHUP` means *look at your configuration
again*, and it has meant that to a daemon for forty years — there is nothing to
encode, which is why this is a signal rather than a socket. A socket would be a
wire format, a version, a permission model and a second thing to keep working,
for one bit.

It costs the loop nothing. The handler does two things, both async-signal-safe:
a relaxed store, and one non-blocking write to the wake pipe that already
exists for finished tasks. So an idle desktop still blocks on its window
system's socket with no timer at all.

**The dangerous part is the pid, not the signal.** `SIGHUP`'s default action is
to terminate, and a pid file left by a crash names a number the kernel will
reuse — so signalling it blindly is a way to kill a stranger's editor. The pid
is therefore checked twice before anything is sent: that the process exists,
and that it is *this program*, by name. A pid file whose process is gone or is
something else is stale, and stale means ignored.

**What can change while a desktop runs is decided by whether it can be done
atomically.** Settings can: they are values behind a shared handle, which is
why `Host::configure` was built to reach a running widget rather than to
restart one. Placement can, by the layer that owns windows.

**Installing and removing can too**, and the shape of that answer is the point.
A widget arriving needs a component compiled, a window created, a swapchain
attached and a placement resolved; a widget leaving needs all of that undone in
the right order. Doing half of either on a signal is how a desktop ends up with
a window nothing draws into — which is why it was refused for as long as it
was, and why the thing that changed is not the risk but the shape of the code.
Each half is now one function that does all of it or none of it, and the two
live in the layer that owns windows. `reload::apply` still refuses to do either:
it reports which names appeared and disappeared, and every widget it names is
still running when it returns.

**The order within a removal is the whole of its correctness.** The renderer's
surface is detached first, because while a surface is attached the next frame
composes into it; the window goes second, because dropping it destroys what
wgpu was presenting to. Reversed, the renderer holds a surface over a window
the server has already forgotten. A widget's outstanding asynchronous work is
dropped with it — ids come from a counter, and a result left behind is a result
the *next* widget would collect.

**A removal is not a deletion.** What a widget saved stays on disk and its
keychain namespace stays where it is; `uniwidged remove --purge` is where
destroying either is asked for, and it asks.

**The monitors can change too**, and nothing the user did causes it. Both
window systems report a layout change — a RandR event mask on the X11 root
window, `wl_output` globals announced and withdrawn on Wayland — and both
arrive as one `Input::OutputsChanged` carrying no list. That is deliberate:
each protocol reports a layout one piece at a time, so a desktop acting on each
piece would place a widget against a layout the server had already moved on
from. It means *ask again*, and the loop asks once however many notifications
arrived, then re-resolves every placement. A session that answers with an empty
list is one that cannot say, and every widget is left exactly where it is —
wiping the list would move all of them to the origin.

### Anything that takes time

A widget must never wait, so nothing slow is a function call. `fetch` and
`run-command` return a **token** immediately; the work happens on a worker
thread; the answer arrives as a `delivered` event carrying that token, and
`take-result` collects it. One mechanism for every kind of slow thing, so that
adding one is a variant rather than a redesign.

Tokens come from one counter shared by the whole desktop, which makes them
guessable — so the handle a widget is given carries its own id, and no host
function can start or collect a task without saying whose it is. A token that is
not yours reads exactly like one that never existed; distinguishing them would
report how many requests the neighbours have made.

The budget is the desktop's: eight tasks at once, sixteen outstanding per
widget. A widget that starts a request on every view is refused rather than
queued, because a queue hides the bug.

The eight workers are **created with the host, not with the work**. Starting a
request happens inside a widget's `view`, on its call deadline, and spawning a
thread is not bounded work — on a loaded machine it exceeds a frame, and the
widget is then interrupted for something the host chose to do. `start` is a
lock, a send and a wake. The workers park on a channel, so an idle desktop pays
nothing for them, and dropping the registry closes the channel, which is the
whole of the shutdown.

**The wake pipe.** §5's loop blocks with no timer at all when nothing is
scheduled, so a task finishing on another thread would otherwise sit in memory
until the user moved the mouse. The loop therefore waits on two descriptors:
the display socket, and a pipe a worker writes one byte to. A pipe rather than
a condition variable because it is the only thing a `poll` set can hold that
another thread can touch without a lock, and rather than a timeout because a
timeout is exactly the polling this project is most careful not to do.

### Moving a widget

A desktop widget is furniture, so putting it somewhere is a drag — and that
makes the pointer ambiguous, because the same press could be a drag or a click
on a button. `uniwidged-host::interaction` is where that is decided, from
things the host already knows:

1. **Locked** — the user has pinned it. Every press is the widget's.
2. **Within six logical pixels of an edge** — a resize. The band wins even over
   a control, because a button on the very edge would otherwise make the widget
   unresizable from that side.
3. **On a node the widget marked interactive** — the widget's.
4. **Anywhere else** — a move.

Rule 3 needs to know which nodes those are, so the compose pass records them
alongside the hit rectangles, for the reason §4 gives about the accessibility
tree: a second walk is a second answer.

Nothing happens until the pointer has travelled four logical pixels, which is
why a click is delivered on **release** rather than on press — before that
there is no way to tell the two apart. Every intermediate rectangle is computed
from the geometry at the moment of the press rather than from the previous one,
so a drag cannot accumulate rounding, and the pointer is grabbed for the
duration, because a drag faster than the compositor otherwise loses it the
moment it leaves the window.

**Placement is stored relative to a named output**, never as a desktop
coordinate and never by index. Indices renumber and coordinates move wholesale
when a screen is plugged in on the left, so a widget remembered either way ends
up somewhere its user did not put it. A widget whose monitor is gone appears on
the primary and *keeps its stored name*, so plugging the monitor back in puts
it back.

### One binary, two window systems

Linux is the only platform where a user can log into a different window system
tomorrow without reinstalling anything, so the choice is made at **run** time in
`platform::linux`, not at compile time. `WAYLAND_DISPLAY` is tried first,
because a Wayland session running XWayland sets both — and reaching a widget
through XWayland means giving up `wlr-layer-shell`, which is the whole
definition of a desktop widget. A failed Wayland connection falls through to
X11 rather than refusing to start, and the start-up log says which happened.

libwayland is `dlopen`ed rather than linked, so the same binary runs on an
X11-only machine that has none installed.

The two shells differ in three ways, and all three are confined to
`platform/wayland.rs`: a Wayland client cannot place a window (a layer surface
is an anchor and margins), is never told where the pointer is on the desktop
(so those coordinates are computed from the placement the client itself asked
for), and is grabbed implicitly for the duration of a press. Above that file
the host's interaction model is identical, which is what stops a widget
behaving differently depending on the session it was started in.

### Before the sandbox: who wrote this?

The sandbox makes running somebody else's code reasonable *once it is running*.
`uniwidged install` asks the question before that, and refuses a widget not
signed by a key the user added to `trusted-keys` — one line per key, in the
configuration directory, meant to be read by the person it protects. There is no
certificate hierarchy and no revocation protocol: everything larger than a file
of keys needs infrastructure this project could not honestly run, and a security
mechanism nobody can inspect is one nobody should rely on.

**The signature covers the component and its manifest together.** The manifest
is what the user is shown and what the runtime enforces, so a signature over the
component alone would let anybody widen a signed widget's capabilities and pass
it on under its author's name. The two are length-prefixed into the digest, and
the digest is domain-separated.

### Containment

* **Memory** — a `ResourceLimiter` cap per instance, declared in the manifest
  and visible to the user.
* **Time** — epoch-based interruption with a deadline on every call. A widget
  that blows it is suspended, marked errored in the UI, and keeps showing its
  last frame. **The desktop never blocks on guest code.**
* **State** — its own data directory, its own keychain namespace, no path out.
  Containment is checked twice: textually on the requested path, and again
  after the filesystem has resolved it, because only the second can see a
  symlink somebody planted in a folder the user can open. Writes land through a
  rename, so a widget interrupted at its deadline leaves a whole file or none.
* **Disk** — two bounds, because they stop two different things. A per-file cap
  bounds what one host call can allocate: without it, a widget could ask the
  host to read a file somebody else put in its directory and make the *desktop*
  allocate a gigabyte, which no guest memory cap would catch. A per-widget
  quota bounds a loop, which is the only other way `fs` reaches a disk — and
  the two report differently, because a write that is too large is a bug in
  that call and a directory that is full is not.

## 7. Graphical quality, concretely

"High graphical quality" is a list of commitments with tests behind them, not
an aspiration:

* **Analytic anti-aliasing** on every vector edge, via Vello's compute
  rasterizer. No MSAA fringing, no stair-stepped rounded corners.
* **True per-pixel alpha** against the real wallpaper on all four platforms —
  DirectComposition, `CAMetalLayer`, an ARGB Wayland surface, a 32-bit X11
  TrueColor visual. This is the difference between a widget that sits *on* the
  desktop and one that sits *in front of* a grey rectangle.
* **Gamma-correct compositing.** Gradients and shadows are interpolated in
  premultiplied linear space. (Ephemeris's `canvas.rs` documents exactly the
  grey-haze artefact that straight interpolation produces between two partly
  transparent stops; Vello does not have it.)
* **Perceptual colour.** Theme colours are authored in Oklch and resolved at
  paint time, so a lightness ramp looks even rather than measuring even. sRGB
  by default, Display P3 where the surface reports it.
* **Fractional scaling handled once**, at the surface, as an `Affine::scale`
  into the presented scene. No widget is DPI-aware; nothing is bitmap-scaled.
* **Text through Parley** — full shaping, bidi and font fallback, hinted and
  gamma-corrected at the real device scale.
* **Real Gaussian drop shadows**, not stacked translucent strokes. The surface
  is larger than the visible glass by the shadow's extent on every side, and
  that margin is `uniwidged-compose`'s to own — a widget author cannot know the
  theme's shadow, and the M0 spike clipped its own shadow by getting this
  wrong.
* **Backdrop blur where the platform gives it** — DWM `SystemBackdrop`,
  `NSVisualEffectView`, KWin/`wp_blur` — with a drawn fallback that degrades
  visibly rather than silently.

Each of these is pinned by a golden-image snapshot test rendered headlessly on
lavapipe, so a regression fails CI rather than shipping.

## 8. Platform matrix

The behaviours that define a desktop widget, and how each platform provides
them. Everything in the first four rows is adopted from Ephemeris; the last row
is new work.

| | Windows | macOS | Wayland | X11 |
|---|---|---|---|---|
| **below windows, above desktop** | `HWND_BOTTOM` forced in `WM_WINDOWPOSCHANGING` | `kCGDesktopIconWindowLevel + 1` | `wlr-layer-shell`, `BOTTOM` layer | `_NET_WM_STATE_BELOW` |
| **never takes focus** | `WS_EX_NOACTIVATE` | `canBecomeKeyWindow → NO` | `keyboard-interactivity = none` | `WM_HINTS.input = False`, no `WM_TAKE_FOCUS` |
| **no taskbar / no Alt-Tab** | `WS_EX_TOOLWINDOW` | `.ignoresCycle` + `.accessory` policy | (the layer has neither) | `_NET_WM_STATE_SKIP_TASKBAR`, `_SKIP_PAGER` |
| **on every desktop / Space** | (bottom-most, always present) | `.canJoinAllSpaces` + `.stationary` | (the layer is per-output) | `_NET_WM_STATE_STICKY` |
| **per-pixel alpha** | `WS_EX_NOREDIRECTIONBITMAP` + DirectComposition | borderless, `opaque = NO`, `hasShadow = NO` | ARGB8888 surface | 32-bit TrueColor visual + own colormap |
| **wgpu attaches via** | `SurfaceTargetUnsafe::CompositionVisual` (DX12) | `SurfaceTargetUnsafe::CoreAnimationLayer` | `RawWindowHandle::Wayland` | `RawWindowHandle::Xlib` — **see §10.1** |

Two notes carried over from Ephemeris, because both were hard-won:

* **`_NET_WM_WINDOW_TYPE_DESKTOP` is the wrong answer on X11.** It means *is*
  the desktop, and window managers that honour it stack the widget beneath the
  desktop icons and stop delivering clicks. `_NET_WM_STATE_BELOW` on an
  ordinary window says what is actually wanted.
* **GNOME's Mutter has no layer shell and has said it will not add one.**
  There the surface falls back to an `xdg_toplevel`: it sits among the windows
  rather than behind them, and it cannot place itself, because Wayland gives no
  client that power. Both facts go in the log at start-up. Degrading quietly
  was the wrong answer in Ephemeris and is the wrong answer here — a widget
  that stops doing the one thing it is for should say which.

There is a fifth target, **headless**, which renders to a texture and reads
back a PNG. It is not a courtesy: it is how every visual test in CI runs.

## 9. Testing strategy

* **Golden-image snapshots** — fixed view trees rendered through compose and
  render, diffed against committed PNGs with a tolerance. Runs on lavapipe, so
  CI needs no GPU and no display. Regenerate intentional changes with
  `UNIWIDGED_UPDATE_SNAPSHOTS=1`.
* **View-tree unit tests** — pure data, no GPU: layout maths, diffing, hit-rect
  computation, a11y tree shape, style resolution.
* **Runtime conformance suite** — one test widget that attempts every
  capability, run under a deny-all policy, asserting each attempt fails. A
  sandbox that is not tested for what it *refuses* is not tested.
* **Scheduler soak** — eight idle widgets for five minutes; assert zero frames
  submitted and the CPU budget of §5 held.
* **Shell tests** — the Wayland path against a nested headless compositor
  (a headless `sway` the suite starts itself) and the X11 path against an
  `Xvfb` it starts itself, both in CI. Windows and macOS shells are exercised
  on their own runners.
* **Synthesised input, on both window systems.** A drag is not a claim anybody
  should make from a unit test, so neither is made: X11's is driven through
  `XTest` and Wayland's through `zwlr_virtual_pointer_v1` — a mouse that does
  not exist, whose events still go through the compositor's own focus and
  coordinate handling before they come back out. Nothing that ships synthesises
  input on either platform; both are test-only dependencies.
* **A running desktop, changed underneath it.** The real binary on a real X
  server with a real device, with a second process installing, removing,
  enabling and disabling widgets while it draws. It is the only place the order
  a window and a swapchain are destroyed in can be wrong, because getting it
  wrong does not fail a type check — it fails on a screen.

## 10. Known risks

### 10.1 X11 per-pixel alpha with a Vulkan swapchain — **largely resolved**

Ephemeris gets a transparent X11 window by picking a 32-bit TrueColor visual
and drawing into it with Cairo. A Vulkan swapchain on X11 usually advertises
only `VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR`, which would put an opaque rectangle
where the wallpaper should be. Mitigations, in the order they will be tried:

1. Query for `PRE_MULTIPLIED` / `INHERIT` composite-alpha support and use it
   where the driver offers it.
2. An EGL/GL path with an ARGB visual — EGL on X11 does honour the visual's
   alpha channel.
3. **Readback presentation.** Vello renders to an offscreen texture exactly as
   it does everywhere else; the pixels are read back and put into the window
   with `PutImage`. Rasterisation is untouched — the same `vello::Scene`, the
   same GPU, the same output — so the golden images still apply. Only
   *presentation* falls back, which is the thing X11 is actually bad at.

   No XRender is needed, which this plan assumed there would be: the window
   already has a 32-bit visual, so the compositor blends its contents exactly
   as it blends a swapchain's.

   This was originally written as "`vello_cpu` from the same scene", and that
   is not possible: `vello` 0.9 builds its scene on `vello_encoding`, while
   `vello_cpu` 0.2 builds one on `vello_common`. They are disjoint types, and a
   scene cannot cross between them. A true no-GPU-at-all path would need
   `uniwidged-compose` to emit into both, and is not planned until the two
   Vello lines converge.

**This was spiked before anything was built on top of it.** On Wayland the
answer is unambiguous: the surface offers `[Opaque, PreMultiplied]`, and
`PreMultiplied` composites correctly through the whole Vello → wgpu →
layer-surface path (`spikes/m0_layer_shell/FINDINGS.md`).

**On X11, path 1 works too** — measured on 2026-08-27 against Xvfb, openbox and
xcompmgr: the surface offered `PreMultiplied`, and the window on screen is the
scene composited over the wallpaper to within one 8-bit unit
(`spikes/m4_x11/FINDINGS.md`).

With one caveat that matters: **that was a software Vulkan driver.** Mesa's
lavapipe presents on X11 through `XPutImage`, which can carry alpha; a hardware
driver on DRI3/Present may still report `Opaque` and nothing else. So path 3 is
built and tested rather than left as a plan — forcing it produces the *same*
numbers, which is this section's claim measured rather than asserted. Path 2
(EGL) is deliberately not built: it would buy GPU presentation on hardware
nobody has measured, and maintaining an EGL/GL path on a guess is worse than
naming the gap.

### 10.2 wgpu does not expect somebody else to drive the surface — **resolved**

A `wlr-layer-shell` surface must `ack_configure` before its first commit, and a
DirectComposition visual is presented through a composition tree the window
does not own. wgpu's surface API looks like it assumes it owns configure and
present timing.

**Measured on 2026-08-27, and it does not.** The M0 spike drives the handshake
itself — create the surface, give it the layer role, commit with no buffer,
receive `configure`, `ack_configure` — and only then hands the surface to wgpu.
wgpu accepts a surface somebody else configured without complaint. See
`spikes/m0_layer_shell/FINDINGS.md`.

One correction came out of it: **wgpu 29 wants the display handle at `Instance`
creation**, not only at surface creation
(`InstanceDescriptor::new_with_display_handle`). Vulkan ignores it; the GL
backend needs it to bind EGL to that `wl_display`. Since GL is the second of
the three X11 mitigations in §10.1, `uniwidged-shell` supplies it from the
start.

### 10.3 GPU cost at idle

One wgpu device plus per-surface swapchains is roughly 30–60 MB of VRAM and a
driver context that Ephemeris's 4 MB RSS does not pay. This is the deliberate
price of §7, and it is paid **once for all widgets** rather than per widget —
which is the entire reason for a single host process. If it proves too high on
laptops, the device can be dropped after a long idle with nothing animating and
rebuilt on the next wake; the surfaces keep their last frame meanwhile.

### 10.4 The component model is still moving

WASI Preview 2 and the component model are stable enough to build on but not
frozen. Wasmtime is pinned to an exact version, the WIT world is vendored in
`wit/`, and the guest ABI gets a version field checked at instantiation —
before `start`, and before anything a widget could do about it.

Two of its constraints already shaped the design rather than merely annoying
it: **no recursive types**, which is why the view tree crosses flat (§6, and
the better shape for it), and **a trapped instance cannot be re-entered**,
which is why a deadline overrun ends an instance instead of being retried.

## 11. Distribution

A framework nobody can get widgets for is a framework with one author. §6 is
about what a widget may do once it is running and §10.3 about who wrote it;
this is the question before both — where does a widget come from?

### 11.1 A registry is a signed file, not a service

`uniwidged-registry-v1` is one TOML document on a static host, listing name,
version, description, capabilities, three URLs and a digest per widget, with a
detached signature beside it. There is no server to run, no account and no API.
That is a deliberate ceiling on ambition: this project cannot honestly operate
infrastructure, and a distribution mechanism that depends on infrastructure
nobody maintains is one that stops working the year after it ships.

Anything larger — search across registries, popularity, moderation — is
somebody else's layer built on top of this file, and can be, because the file
is public and its format is a page long.

### 11.2 Two signatures, by two people, checked separately

| Claim | Signed by | Key kept in |
|---|---|---|
| *this is the list I published* | the registry publisher | `registries.toml`, pinned when the registry was added |
| *this is the widget I wrote* | the widget's author | `trusted-keys`, added with `uniwidged trust` |

Neither implies the other, and the digests are domain-separated so that a
signature over one can never be replayed as a signature over the other. The
consequence is the property that matters: **a registry cannot install anything.**
It can offer, and the machine's own trusted keys still decide.

The index digest is `sha256` over the component *and* its manifest — the same
digest a widget's signature covers, for the same reason (§10.3): a digest over
the component alone lets somebody keep the bytes and widen the manifest, and
the manifest is the whole of what a user is asked to approve.

### 11.3 What an index promises is checked against what arrives

An entry lists the capabilities a widget declares, because a person choosing
what to install reads that list before anything is downloaded. So the manifest
that arrives must **agree** with the entry that offered it — same name, same
version, same capabilities — or the install is refused even when every
signature is valid and every hash matches. Otherwise a listing is a place to
put one sentence in front of somebody and run another.

### 11.4 The unattended half

`uniwidged update` is the one command that installs something nobody looked at,
so it is the one command that may not widen a permission. An upgrade whose
manifest asks for anything the installed grant does not already cover is
reported and not applied; taking it means running `add`, where the whole list
is printed. Everything else about the widget — where it sits, what it was
configured with, whether it was switched off — is carried across, because a new
version of a widget is not a new widget.

## 12. Provenance and licensing

uniwidged is MIT OR Apache-2.0. `uniwidged-shell` derives from Ephemeris
(GPL-3.0-or-later) under a relicence by its sole copyright holder; see
[`NOTICE.md`](https://github.com/JosunLP/uniwidged/blob/main/NOTICE.md) for the verification. This is not a detail: a
framework's licence is the licence its users inherit, and a permissive one is
what lets a widget author choose their own.
