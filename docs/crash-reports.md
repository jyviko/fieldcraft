# Crash reports, the log file and Report a Bug

## Privacy

No crash report, log line or model is uploaded automatically. Crash reports and the log stay on
this computer (anonymous usage, if you agreed to share it, never carries them: see
[privacy.md](privacy.md)). The only way they leave it is **Help › Report a Bug** (or **Report It…** after a crash), which opens a GitHub issue
form in your browser; you read it there and submit it, or not.

**Help › Crash Reports…** lists every saved report (when, what, how many times), each with
**Report…**, **Copy Path** and **Delete**, and **Delete All** (`diagnostics.crash_reports`,
`diagnostics.crash_reports.delete`).

What a bug report carries:

| Always (you can edit it) | Only with **Share my session** | Only with **Include the model file** as well |
|---|---|---|
| the version and OS, what happened (the last error by default), for a crash its message, place and backtrace top; your home folder is shown as `~` | the commands that built the model (`fieldcraft-session.json`, replayable with `fieldcraft-cli run`); they can name your files | the model (`model.fieldcraft`), which you attach yourself |

The form shows exactly what will be filled in before you click **Open GitHub Issue**. GitHub
refuses long new-issue links, so a session that does not fit goes out as a file instead: it is
saved to `bug-reports/<time>/` in the data folder below and copied to the clipboard, and the issue
text says to attach or paste it. The model is always a file you attach.

## Where things are

The app's data folder holds `recovery/`, `crashes/`, `logs/` and `bug-reports/`:

| System | Folder |
|---|---|
| macOS | `~/Library/Application Support/FieldCraft` |
| Linux | `~/.local/share/fieldcraft` (or `$XDG_DATA_HOME/fieldcraft`) |
| Windows | `%LOCALAPPDATA%\FieldCraft` |

Runs driven by automation (`--control`, or `CI` set) use `fieldcraft-automation/` in the
temporary folder instead, so test runs never reach your folders. `FIELDCRAFT_CRASH_DIR` moves the
crash folder, `FIELDCRAFT_RECOVERY_DIR` the recovery folder. The browser build keeps no crash
folder and logs to the browser console.

## The log file

`logs/fieldcraft.log`: one line per record, `time LEVEL [pid] target: message`. Past 2 MB it
becomes `fieldcraft.log.1` and a new one starts. Warnings and errors also go to the standard
error stream.

## Crash reports

`crashes/<YYYYMMDD-HHMMSS>-<signature>/`:

- `report.json` (`"format": "fieldcraft-crash/1"`): `kind` (`command_panic`: a command hit an
  internal error, caught, the model left unchanged; `recovered_panic`: one caught inside a command;
  `panic`: an internal error outside commands; `unclean_exit`: the app ended without a clean exit
  — an abort, a stack overflow, a crash in a graphics driver, the process being killed outright
  (SIGKILL) — found at the next start), `message`, `location`, `thread`, `backtrace`, `app`, `system`, `command`,
  `recent_commands` (up to 32), `last_activity` (the last control request, such as
  `control ui.capture`), `base`, `count`, `first_seen`, `last_seen`, `notes`
- `model.fieldcraft`: the model the replay starts from
- `replay.json`: `{"commands": [{command, params}, …]}`
- `log.txt`: the crashed run's lines of the log

The next start after a crash asks "FieldCraft closed unexpectedly": **Report It…** opens Report a
Bug for that report.

### For agents

| From | List | One report | Replay |
|---|---|---|---|
| Command (MCP `execute`, control channel, scripts) | `diagnostics.crash_reports` | `diagnostics.crash_report {id}` | |
| CLI | `fieldcraft-cli crashes [--json]` | `fieldcraft-cli crashes show <id>` | `fieldcraft-cli crashes replay <id>` |

`crashes replay` opens the report's model, runs its commands (files they write can only go to a
scratch folder) and says whether the same internal error happens again. `crashes dir` prints the
folder, `crashes clear [<id>]` deletes reports. `diagnostics.bug_report` returns the draft (link,
fields, attachments) without opening anything; over the control channel `ui.start
diagnostics.bug_report` opens the form, `ui.prompt {"answer": "set", …}` sets its fields, and
`ui.inspect` shows the draft. Only a click opens the browser.

**Check that reporting works:** with `FIELDCRAFT_CRASH_TEST=1`, `diagnostics.crash_test {mode}`
crashes on purpose: `panic`, `recovered` or `abort` (reported at the next start).

A request to terminate is no crash: on Linux and macOS, SIGTERM (`kill PID`) and SIGINT (Ctrl-C)
close the window as **Close** does (unsaved work is autosaved for recovery, the
journal ends, no report). A second signal ends the app at once.

### How it stays cheap and small

The app keeps a journal of the window's top-level commands (`crashes/sessions/*.jsonl`, at most
512 KB, two small writes per command that changes the model, none for commands that only read),
restarted from each autosave and each model opened. A clean exit deletes it; it is locked while its
app runs, so a journal nobody holds belongs to an app that died. A crash is identified by its
place and message, numbers ignored: repeats only raise `count`. At most 5 new reports per run,
20 reports, 100 MB, 90 days; a model over 20 MB is not copied. Tests, the CLI and the browser build
never install the hook.

Adapted from SolidCraft's crash reports (MIT OR Apache-2.0, see ATTRIBUTION.md).
