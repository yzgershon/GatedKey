<div align="center">

# GatedKey

**Point any key on your keyboard at any app.**

No scripting. No reboot. No 200MB suite.

</div>

---

## What it does

You have keys you never use. The Copilot key. A macro key. F13 through F24 on a
fancy keyboard. Print Screen. GatedKey lets you point any of them at something
you actually want:

- **Open an app** (including Start Menu shortcuts, not just `.exe` files)
- **Open a website**
- **Open a folder**
- **Run a command**

Press the key, pick the thing, done. It takes about fifteen seconds and nothing
about it involves writing a script.

## Reclaiming the Copilot key

This is the reason most people show up, so here it is directly.

The Copilot key is not a special key. Per Microsoft's keyboard spec it emits an
ordinary chord: **Left Shift + Left Win + F23**. `F23` is used precisely because
no physical keyboard has one, so nothing else collides with it.

That means it can be caught and replaced like any other shortcut. GatedKey
catches it, swallows it so Copilot never opens, and runs whatever you told it to
instead.

> **Why not just use the setting in Windows?**
> Settings → Personalization → Text input → "Customize Copilot key" only lists
> **MSIX-packaged, signed** apps. Most software you actually use, including most
> Electron apps and anything installed with a normal installer, will never appear
> in that dropdown.

## Install

Grab the latest installer from [Releases](https://github.com/yzgershon/GatedKey/releases).
Windows 10 and 11, x64 and ARM64.

GatedKey has to be running for your keys to work, because a keyboard hook lives
and dies with its process. It sits in the tray and starts with Windows if you
tell it to.

## Is this a keylogger?

It is the fair question to ask about anything that installs a keyboard hook, so
it gets a real answer rather than a reassuring one.

**A low-level keyboard hook does see every key.** There is no way to build this
category of app without one. Windows offers no "only tell me about F23" API. So
the question that matters is not what it *can* see, it is what it *does* with
what it sees.

GatedKey's answers, all of which you can verify in
[`src-tauri/src/hook.rs`](src-tauri/src/hook.rs):

1. **No keystroke is ever written to disk.** Keys that do not match one of your
   bindings are compared against them and dropped on the spot. Nothing is
   buffered, counted, or logged. There is no log file to leak.
2. **No network access at all.** The app makes zero outbound connections. There
   is no telemetry, no update ping, no crash reporter. Block it at the firewall
   and nothing changes.
3. **The config file holds bindings and nothing else.** Plain JSON, in your own
   AppData folder, readable in a text editor.
4. **The binary is small enough to actually audit.** The whole keyboard hook is
   one file of a few hundred lines.

If any of that ever stops being true, it is a bug, and a serious one.

**Your antivirus may still flag it.** A keyboard hook plus process launching is
the textbook signature of malware, and unsigned binaries trip SmartScreen. That
is a real cost of this category, not a sign something is wrong. Building from
source is always an option if you would rather not trust a binary.

## How it works

The hook is installed on the **main thread**, the one running the application's
message loop. This looks like the wrong choice and is not.

A low-level hook is dispatched on the thread that installed it, and that thread
must be pumping messages. Installing it on a dedicated worker thread with its
own `GetMessage` loop, which is the tidier-looking design, fails in the worst
possible way: `SetWindowsHookExW` returns a valid `HHOOK`, the loop runs, and the
callback is simply never invoked. The hook reports as installed and does nothing.
If you are reading this planning to move it off the main thread, that is why it
is not there already.

Two further constraints shape the design:

**The callback runs on every keystroke on the machine, and it has a deadline.**
If it does not return within `LowLevelHooksTimeout` (300ms by default) Windows
silently removes the hook. No error, no crash, the app just quietly stops
working. So the callback never launches anything, never waits on a lock, and
never allocates on the non-matching path. It compares against a snapshot, posts
to a channel, and returns. A worker thread does the actual work.

**Swallowing a key is not one event.** When a binding matches, both the keydown
and its keyup are swallowed, because a stray keyup with no matching keydown
confuses applications. And if the chord includes the Win key, GatedKey injects a
harmless Ctrl tap so Windows does not pop the Start menu on release, which it
otherwise would, since from the shell's point of view Win was pressed and
released with nothing in between.

## Troubleshooting

If a key does nothing, run GatedKey with `GATEDKEY_DEBUG=1` set and watch stderr.
It will report whether the hook installed, how many bindings loaded, and a
running count of how many times the hook has fired.

That count is the useful one. A hook can install successfully and still never be
invoked, and those two failures look identical from the outside. If the count
climbs when you type, the hook is alive and the problem is your binding. If it
stays at zero, the hook is not receiving input at all.

Diagnostics can only ever report modifiers and F13 through F24. Ordinary typing
is unreportable by construction, so turning debugging on cannot turn GatedKey
into a keylogger.

## Compared to the alternatives

| | Scripting? | Launch apps? | Reboot? | Download |
|---|---|---|---|---|
| **GatedKey** | No | Yes | No | **0.96MB** |
| PowerToys | No | Yes, buried | No | ~200MB suite |
| AutoHotkey | Yes | Yes | No | small |
| SharpKeys | No | **No**, key to key only | **Yes** | small |

Those sizes are measured, not estimated: a 2.6MB executable, a 0.96MB installer.
GatedKey is small because it uses the WebView2 that already ships with Windows
instead of bundling a browser engine, and because the whole app is a keyboard
hook and a list.

## Build from source

You need [Rust](https://rustup.rs/), [Node](https://nodejs.org/) (for the Tauri
CLI only), and the MSVC build tools.

```bash
git clone https://github.com/yzgershon/GatedKey
cd GatedKey
npm install
npm run dev      # run it
npm run build    # produce an installer
```

The frontend is plain HTML, CSS and JavaScript with **zero runtime npm
dependencies**. `npm install` pulls the Tauri CLI and nothing else, so there is
no dependency tree to audit on that side.

Run the tests with:

```bash
cd src-tauri && cargo test
```

## Roadmap

- Focus an already-running app instead of launching a second copy
- Remap a key to a different key, not just to an action
- Per-app bindings (a key that does one thing in one program, another elsewhere)
- Import and export config
- Code signing, once there is enough usage to justify a certificate

## License

MIT. See [LICENSE](LICENSE).
