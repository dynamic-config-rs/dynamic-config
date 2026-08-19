# Migrating a Configuration Schema

The field was `url`, the new code wants `endpoint`, and the store still
serves the old document. What happens, and the choreography that makes
it a non-event. **There is no migration engine here, on purpose** —
deserialize-fails→last-known-good already bounds the blast radius, and
a migration DSL would be a second schema language to get wrong.

## What happens with no plan

New code deploys; the old document fails to deserialize; the process
that was running keeps serving last-known-good; a process that
*restarts* fails init (correctly — see [Readiness](readiness.md)).
So: nothing burns instantly, and nothing converges either. Plan.

## The tools already in the box

- **Aliases** — the built-in answer for renames:
  `builder.alias("url", "endpoint")` lets the old spelling in the
  document fill the new field. Ship it in the same release that renames
  the field, delete it two releases later. Most migrations end here.
- **`Option<T>` + default** — the answer for additions: a new field
  the old documents lack must be optional or defaulted for one
  transition window.

## The choreography for shape changes

When a rename will not do (a field splits, a table restructures), run
the three-step dance — consumers first, always:

```text
1. consumers accept BOTH        (enum or untagged struct over V1|V2,
                                 or schema_version: u32 + branch)
2. producers switch to V2       (the store's document changes)
3. consumers drop V1            (next release, delete the branch)
```

A `schema_version` field costs one line and turns "which shape is
this?" from inference into a statement:

```toml
[app]
schema_version = 2
```

## The one rule

Never make step 2 before step 1 is fully deployed — the store is
shared state, and a document only V2-readers understand bricks every
V1 process that restarts. This is the same contract as any rolling
data migration; configuration is data.
