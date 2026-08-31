# uniwidged

**A cross-platform desktop widget framework, built for graphical quality and
for costing nothing when nothing is happening.**

One host process owns one GPU device and every widget's surface. Widgets sit
below your windows and above your wallpaper, composited per-pixel against it —
not floating in a grey rectangle. They are WASM components: sandboxed,
installable at runtime, and written in any language that targets the component
model.

> **Status: scaffold, with the load-bearing assumption proven.** The
> architecture is settled and written down, the vocabulary crates are real and
> tested, and the platform layers are documented skeletons.
>
> **Milestones M0 through M4 are done, M7 in full and M8 in part.** A
> `wlr-layer-shell` surface on the bottom layer really does back a wgpu
> swapchain with per-pixel alpha, and the widget really does stay below
> ordinary windows. `uniwidged-render` drives it: one
> device, many surfaces, an idle frame costing zero submits, and a headless
> render that matches the on-screen one to within one 8-bit unit.
> `uniwidged-compose` turns a view tree into that scene — layout, text, theme,
> icons, hit rectangles and an accessibility tree, all from one pass — with
> fifteen golden images behind it. And widgets really are sandboxed WASM
> components: `widgets/conformance` is built for `wasm32-wasip2`, instantiated
> by the real runtime, and twenty-four tests assert what the sandbox refuses —
> and, since every gate acquired something behind it, what it serves.
> And it runs on X11 as well as Wayland — the risk `ARCHITECTURE.md` §10.1
> named, measured rather than assumed, with the documented fallback built and
> forced so it stays honest. See
> [`spikes/m0_layer_shell/FINDINGS.md`](spikes/m0_layer_shell/FINDINGS.md) and
> [`spikes/m4_x11/FINDINGS.md`](spikes/m4_x11/FINDINGS.md).
> And it runs: `uniwidged run` starts a real desktop on X11 with sandboxed WASM
> widgets in it, drawing, responding to clicks, restarting when one fails, and
> submitting nothing at all when nothing is happening. Behind the capability
> gates is a widget's own data directory, which
> it cannot leave even through a symlink; dates and times in the machine's
> locale; text measured against the host's real fonts in the theme it is about
> to be drawn in; the machine's CPU load, sampled without ever blocking a
> widget's deadline; declared commands, run asynchronously so that a widget
> never waits; and HTTPS to a granted host — where a redirect off that host is
> refused, because otherwise a grant for one name is a grant for wherever that
> name points.
>
> **And now the last one.** `secrets` is served: an encrypted keychain, one
> namespace per widget with no argument through which to name another's, and a
> description of what that encryption does and does not buy which `install`
> prints at the moment somebody is asked to approve it — because "store
> passwords in your keychain" is a sentence whose weight depends entirely on
> what the keychain is. Every capability this framework defines now has
> something behind it, and the conformance suite asserts the empty list from
> the widget's side, so one that stops being served fails a test.
>
> **And a widget you did not write is checked before it runs.** `install`
> refuses one that is not signed by a key this machine was told to trust, and
> the signature covers the *manifest* as well as the component — otherwise
> somebody could widen what a widget may do and keep a signature its author's
> users recognise. The
> workspace builds from crates.io alone: no sibling checkout, no private
> repository, `git clone && cargo test`. Widgets can be dragged, resized and
> pinned, and they remember which monitor they were on — measured against a
> real X server with synthetic pointer input, not asserted — including across
> two monitors, where a widget stored against the right-hand one opens where
> the right-hand one is, survives being unplugged without forgetting which
> screen it belongs to, and comes back exactly where it was when it is plugged
> in again. It starts with the session, it will not overwrite an autostart
> entry somebody wrote by hand, and a widget granted `fs` can no longer fill a
> disk one file at a time.
>
> **And widgets have settings.** A widget declares what it can be configured
> with — a type, a range, a default — the host validates before the widget is
> started, and `uniwidged configure` is where a wrong value is refused. So a
> widget reading a number never has a branch for somebody typing `banana`, and
> `Event::Configured`, which had been in the interface file since the first
> commit with nothing to produce it, now fires.
>
> **A running desktop is told.** `uniwidged configure`, `move` and `lock` reach
> a desktop that is already running — one `SIGHUP`, one byte down the wake pipe
> that already exists, and a widget reconfigured without being restarted. And a
> link a widget draws now opens: `https` only, to a host the manifest declared,
> checked at the click rather than when the view was built.
>
> **And widgets arrive and leave while it is running.** `install`, `remove`,
> `enable`, `disable` and `update` all reach a desktop that is on the screen:
> a widget appears where its placement says, or goes, taking its window, its
> swapchain and its outstanding work with it — and leaving what it saved, which
> only `remove --purge` deletes. An upgrade is applied live too, because a new
> version under the same name is invisible to a diff of names and is exactly
> what `update` produces. Measured against the real binary on a real X server
> with a second process changing the configuration underneath it, because the
> order a window and a swapchain are destroyed in is not something a type check
> can be wrong about.
>
> **And it notices the monitors changing.** A screen plugged in, unplugged,
> moved or resized reaches the loop on both window systems, and every widget is
> put back where its placement says — which is what makes "remembers which
> monitor it was on" true of a laptop that is docked while it is running, and
> not only of one that is restarted afterwards.
>
> **And now there is somewhere to find widgets.** A registry is one signed TOML
> file on a static host — no accounts, no API, no server to run — and
> `uniwidged search · add · update` read it. Two signatures by two people are
> checked separately: the *index* is signed by whoever publishes the list, with
> its key pinned when the registry is added, and each *widget* by whoever wrote
> it, against the trusted keys this machine already had. Neither implies the
> other, so a registry can offer and never install: the machine's own keys
> still decide. What a listing claims is checked against the manifest that
> arrives — a download whose manifest asks for more than the entry showed is
> refused with every signature valid — and `update`, the one command that
> installs something nobody looked at, will not widen a permission: it says so
> and sends you to `add`, which prints the whole list. It is all driven against
> a real TLS server in the tests, including an index edited after it was signed
> and a component swapped after it was listed.
>
> **And it packages.** `packaging/deb/build.sh` builds a `.deb` with `dpkg-deb`
> and nothing else, and CI builds it and then looks inside: the binary, the
> manual page and the desktop entry are there, the version in the package is
> the version in the workspace, and the binary that comes out of it runs. What
> is still missing is named in [`ROADMAP.md`](ROADMAP.md) rather than glossed
> over — the Windows and macOS platforms, and the packaging that waits on them,
> because packaging a binary that prints "this build has no shell for the
> current platform" would be packaging nothing.
>
> **And it says what it is doing, after the terminal has gone.** A desktop
> started with the session has no terminal, so everything it prints goes to a
> log beside the configuration as well — a megabyte, rotated once — and
> `uniwidged log` reads it, `-f` follows it. `uniwidged status` answers the
> question `list` never could: not what the file asks for, but what is on the
> screen — which widget is drawing, which one the desktop gave up on after
> three failures, where each actually is and which shell the session gave it.
> A desktop killed outright leaves both files behind, and neither is believed
> until the pid beside them is checked to be alive and to be this program. And
> one widget that will not start — a deleted component, a manifest somebody was
> halfway through editing — no longer empties the whole screen: it is named,
> and the rest start.
>
> **And a widget can draw a chart.** `sparkline(values)` is the one node that
> carries a series rather than a value, and it is as far as this framework goes
> towards a canvas: the widget hands over numbers, the host draws the line, the
> area and the marker on the latest point, in the theme's stroke and the node's
> accent — so a screen of charts by different authors is still one chart style.
> Two hundred and fifty-six points at most, refused past that at the boundary
> like everything else. `metrics` now answers for memory as well as the CPU,
> under the one grant, and `widgets/system-load` shows a minute of history under
> its meter. Four more golden images, and the shell completes widget names:
> `uniwidged completions bash|zsh|fish` is written by the binary from the
> commands it actually has, and the packages install it.
>
> **Both Linux window systems.** A Wayland session gets a `wlr-layer-shell`
> surface on the bottom layer; an X11 session gets a 32-bit
> `_NET_WM_STATE_BELOW` window; the binary picks at start-up and says which. A
> test starts its own headless `sway`, creates the surface `uniwidged run`
> creates, and checks what only a compositor can confirm.
>
> And somebody else can write one. `uniwidged new` scaffolds a widget that
> already builds, draws and responds; [the authoring
> guide](docs/writing-a-widget.md) is written; and there are three reference
> widgets, one per served capability. A test runs the scaffold, builds what it
> wrote for `wasm32-wasip2`, instantiates it in the real runtime and clicks its
> button — so the guide's first four commands are checked on every change
> rather than believed.

![Two sandboxed WASM widgets running on a desktop](docs/images/desktop.png)

*Not a mockup: `uniwidged run` on an X11 session, screenshotted. Two WASM
components, each in its own desktop-layer window with its own swapchain, drawn
from one GPU device. The right-hand one is the conformance widget, reporting
that every capability it tried was refused.*

## Where it comes from

uniwidged is the union of two projects, and both halves are load-bearing.

[**Ephemeris**](https://github.com/JosunLP/Ephemeris) worked out how a window
becomes a *desktop widget* rather than an application window — on Windows,
macOS, Wayland and X11, with all the flags and protocols that implies, and all
the ways each platform gets it wrong. That is `uniwidged-shell`.

[**Ferrum UI**](https://github.com/j-pfalzgraf/ferrumui) is a GPU-rendered UI
stack on Vello, wgpu, Parley, taffy and AccessKit, and it is where the
rendering, layout and text techniques above the surface come from.

Both are **adopted, not depended on**. This workspace has no path dependency on
a sibling checkout and nothing outside crates.io: `git clone && cargo test`
works on a machine with neither project on it. `NOTICE.md` records what came
from where, and every adapted file says so in its own module documentation.

What is new is the seam between the two — handing a desktop-layer surface to
wgpu instead of to Cairo — and the sandbox boundary that lets somebody else
write a widget.

## What makes it fast

There is no idle loop and no idle frame. Every widget answers "when should I be
called again?", the scheduler sleeps until the earliest of those, and between
them the process blocks on its event sources with no timer at all. Widgets
whose deadlines fall close together share one wake-up — eight clocks cost one,
not eight — and a widget whose view tree has not changed produces no GPU work
whatsoever.

Ephemeris reached 0.03 s of CPU per 40 s of idle this way with three hand-rolled
timers. uniwidged generalises it to N widgets that have never heard of each
other.

## What makes it look right

Analytically anti-aliased vector rendering, gamma-correct compositing,
perceptual colour in Oklch, real Gaussian shadows, full text shaping with bidi
and font fallback, fractional scaling resolved once at the surface, and true
per-pixel alpha against the actual wallpaper on all four platforms.

Every one of those has a golden-image test behind it, rendered headlessly in CI
on a software rasterizer. See [`ARCHITECTURE.md`](ARCHITECTURE.md) §7.

## Writing a widget

```sh
uniwidged new my-widget && cd my-widget
cargo build --release --target wasm32-wasip2
uniwidged install target/wasm32-wasip2/release/my_widget.wasm && uniwidged run
```

That is a working widget on your desktop. [**The authoring
guide**](docs/writing-a-widget.md) is the rest of it, [**the API
reference**](docs/api-reference.md) is every function and limit on one page, and
[`widgets/clock`](widgets/clock), [`widgets/system-load`](widgets/system-load),
[`widgets/counter`](widgets/counter), [`widgets/uptime`](widgets/uptime) and
[`widgets/vault`](widgets/vault) are five worked examples, one per capability.

You describe; the host decides what it looks like. That is what makes a screen
full of widgets by different authors look like one product.

```rust
use uniwidged_guest::prelude::*;

#[derive(Default)]
struct Clock;

impl Widget for Clock {
    fn view(&mut self, cx: &Cx) -> View {
        let now = now_seconds();
        panel(NodeId::named("clock"))
            .child(text(cx.format_time(now).unwrap_or_default()).role(Role::Display))
            .child(text(cx.format_date(now).unwrap_or_default()).role(Role::Caption).muted())
            .view()
    }

    // The top of the next minute — not "in 60 seconds", so the clock stays
    // right across a suspend, a timezone change and an NTP step.
    fn wake(&self) -> Wake { Wake::At(next_minute()) }
}

uniwidged_guest::export!(Clock);
```

Every host call returns a `Result`, and the refusals are distinguished: a
capability the manifest never asked for answers `NotGranted`, and one the user
approved that this host cannot serve answers `Unavailable`. A user can decline
anything, and a widget that disappears when they do is a worse answer than one
that shows its last value and says it is stale. Off-target the whole thing still compiles and its
tests still run — `export!` expands to nothing there — so a widget's logic can
be tested without a WASM toolchain.

No colours, no fonts, no pixels, no coordinates. A widget names
`Role::Display`; the theme decides what that is. A widget names
`Accent::Critical`; the theme picks a red that is legible against *this*
wallpaper at *this* contrast setting.

Capabilities are declared and host-scoped:

```toml
# widget.toml
name = "weather"
capabilities = ["timer", "net:api.open-meteo.com", "fs:cache"]
memory-mb = 32
```

`net:api.open-meteo.com`, not `network` — because "this widget talks to
api.open-meteo.com" is something a user can agree to, and "this widget uses the
network" is not. `link:open-meteo.com` is a *different* line, because "talks to
a server" and "can send you to a web page" are different things happening to a
user.

## Running it

```sh
uniwidged install my_widget.wasm     # tells a running desktop, if there is one
uniwidged run                        # or start one; `uniwidged autostart enable` for next time
uniwidged status                     # what is on the screen right now
uniwidged log -f                     # what the desktop is saying, terminal or not
uniwidged configure system-load --set warn-at=0.8
```

`status` and `list` answer different questions — what is on the screen, and what
the file asks for — and a widget that is in the second and not the first is
exactly the case somebody running either is trying to explain, so `list` says
so beside the name. `uniwidged completions bash` (or `zsh`, `fish`) makes the
shell complete all of it, widget names included.

## Finding a widget

```sh
uniwidged registry add https://widgets.example/index.toml --key 3d40…
uniwidged search clock
uniwidged add clock
uniwidged update
```

A registry is one signed TOML file on a static host. The key you pin says *who
publishes the list*; whether a widget is installed is still decided by
`trusted-keys`, where it always was. A registry cannot vouch a widget past
that: downloading is not trusting.

Publishing one is `uniwidged registry publish --key <yours> --base-url <where>
*.wasm`, and then uploading what it wrote. Both halves are
[**`docs/registries.md`**](docs/registries.md).

## Layout

```
crates/
  uniwidged-core       geometry · colour · ids · events · the wake model
  uniwidged-view       the view tree that crosses the sandbox boundary
  uniwidged-compose    view → layout → text → Vello scene · hit rects · a11y
  uniwidged-render     one wgpu device, many surfaces, swapchain or readback
  uniwidged-shell      desktop-layer surfaces: wayland · x11 · win32 · appkit
                       (`platform::linux` picks between the first two at run time)
  uniwidged-runtime    the WASM sandbox: capabilities, limits, lifecycle
  uniwidged-guest      the SDK widget authors compile against
  uniwidged-host       the daemon: scheduler, configuration, policy
  uniwidged-cli        `uniwidged run · status · log · new · check · install · …`
wit/uniwidged.wit      the host/guest contract
widgets/clock          the reference widget for `locale`
widgets/system-load    the reference widget for `metrics`, and for settings
widgets/counter        the reference widget for `fs`, and a restart it survives
widgets/uptime         the reference widget for `command`, and the async shape
widgets/vault          the reference widget for `secrets`, and a credential
                       that never reaches the view
widgets/conformance    a widget that tries everything, so the sandbox can be
                       tested for what it refuses
docs/writing-a-widget.md  the guide
docs/api-reference.md     every function, type, limit and refusal, on one page
docs/registries.md        finding a widget, and publishing one
packaging/                a `.deb` that CI builds and looks inside, a PKGBUILD,
                          and the manual page
spikes/m0_layer_shell  the proof that the whole thing is possible
spikes/m4_x11          the same question on X11, where it was least certain
```

## Building

```sh
cargo test --workspace
```

That is the whole of it: every dependency is on crates.io, and there is no
sibling checkout to arrange.

Some tests need more than a compiler, and say so rather than skipping quietly:

| | |
|---|---|
| the golden images | a Vulkan driver — `mesa-vulkan-drivers` supplies a software one. Without it they skip; `UNIWIDGED_REQUIRE_GPU=1` turns that into a failure, and CI sets it. |
| the sandbox conformance suite | `rustup target add wasm32-wasip2`. It builds the conformance widget itself, so there is nothing to skip. |
| the layer-shell tests | `sway`. They start their own headless compositor, so no session is needed; without it they skip, and `UNIWIDGED_REQUIRE_WAYLAND=1` makes that a failure. CI sets it. |
| the M0/M1 spike | a wlroots compositor and `grim`; `spikes/m0_layer_shell/run.sh` starts a headless sway if none is running. |
| the HTTPS tests | `openssl`, to make a certificate for the TLS server they start themselves. No internet is used or needed. Without it they skip, and `UNIWIDGED_REQUIRE_NET=1` makes that a failure. |
| the registry tests | `openssl`, as above: they start a TLS server, serve a signed index over it and drive the real binary against it. Same bargain — skipped without it, a failure under `UNIWIDGED_REQUIRE_NET=1`. |
| the pointer tests | `Xvfb`. They start their own nested X server, so no session or display is needed; without it they skip, and `UNIWIDGED_REQUIRE_X11=1` turns that into a failure. CI sets it. |
| the two-monitor tests | `Xvfb` and `xrandr`, which declares two virtual RandR monitors over the nested server's screen — the same protocol path a docked laptop uses. Same bargain: skipped without them, a failure under `UNIWIDGED_REQUIRE_X11=1`. |

## Licence

MIT OR Apache-2.0, at your option — deliberately, because a framework's licence
is the licence its users inherit, and a widget author should be free to choose
their own. The shell layer derives from Ephemeris (GPL-3.0-or-later) under a
relicence by its sole copyright holder; the verification is in
[`NOTICE.md`](NOTICE.md).
