# Change Notification, Across the Languages

One contract, three surfaces, written once — the Python and Node
books link here rather than restating it, so the three cannot drift.

## The table

| question | Rust | Python | Node |
|---|---|---|---|
| await the next install | `changes().changed()` | `changed_async()` / `async for changes()` | `for await (const doc of config.changes())` |
| installs **and** refusals | `events().next_event()` | `events()` | `events()` |
| callback on install | `on_reload` / `on_reload_with` | `on_reload` | `onReload` / `onReloadAsync` |
| callback on refusal | `on_reload_failed` | via `events()` | via `events()` |
| triggering | edge: a wake per state change, never a queue replay | same | same |
| install delivery | **latest-wins**: ten installs while nothing awaited collapse to the newest | same | same |
| refusal delivery | never collapsed *into* a success — refusal-then-install yields both, refusal first | same | same |
| slow consumer | misses intermediate installs, never the newest; misses no refusal *kind* transition | same | `onReloadAsync` picks the policy: `latest` (abort in-flight), `serial` (FIFO all), `every` (fire-and-forget) |
| lossless stream? | **No, by design** — see below | no | only `serial`, and only for the hooks it guards |

## Why delivery is edge-triggered and latest-wins

A configuration subscriber wants *the newest state*, not a history: a
pool resizing to an intermediate size it will immediately resize away
from is work, not correctness. So delivery is edge-triggered and
collapsing — `changes()` resolves with whatever is current when the
waiter runs, and generations may skip. Code that needs a total order
reads `generation` (monotonic); code that needs an event *log* wants a
message bus, which this is not and will not become.

Refusals are the exception to collapsing in one direction only: a
refusal is never swallowed by the install that races it, because "the
document was refused, then fixed" is operationally different from "the
document changed". The reverse collapse (two refusals while nothing
awaited → one wake) does hold — the *count* is on
`status().consecutive_failures` and the monotonic `refusals()` counter,
not in the stream.

## What a failure delivers

`FailureStatus`: the category and the key path, never a message and
never a value — the same discipline as every diagnostic surface here.
Node's `latest / serial / every` backpressure vocabulary applies to
failure events exactly as to installs.

## Before the first install

A handle created before `init()` has seen nothing, so the initial
install is its first event in every language — "wake me when
configuration exists" is contract, not accident.
