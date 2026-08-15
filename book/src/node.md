# Node.js Bindings

The Node.js binding has **a book of its own**:

### → [dynamic-config for Node.js](https://dynamic-config-rs.github.io/node/)

```sh
npm install dynamic-config-node
```

```ts
import { DynamicConfig, zodValidator } from "dynamic-config-node"
import { z } from "zod"

const Database = z.object({ host: z.string(), port: z.number().default(5432) })

const db = await new DynamicConfig({ key: "db", validate: zodValidator(Database) })
  .file("config.toml")
  .env("APP_")
  .initAndCurrent()
//    ^? { host: string; port: number }
```

**Rust resolves, your schema validates, JavaScript reads a cached object.**
One prebuilt binary per platform through Node-API, so the same binary
serves every Node version the package claims and nothing compiles at
install time.

## Why it is a separate book

The same reason the Python one is: the reader is a different person. A
Node programmer arriving from npm should not land in a sidebar whose first
twenty entries are Rust, and a Rust programmer carries no chapters about
an event loop. The store crates stay in this book — a Consul chapter is
read by whoever read the [builder tour](builder-tour.md).

## What is in it

| Chapter | What it answers |
|---|---|
| [API Reference](https://dynamic-config-rs.github.io/node/reference.html) | Every method, every argument, every default |
| [Schemas](https://dynamic-config-rs.github.io/node/schemas.html) | Zod, Ajv, a function of your own, and no schema at all |
| [Watching & Hooks](https://dynamic-config-rs.github.io/node/watching.html) | The watcher, `onReload`, `onChange`, and the rejected edit that changes nothing |
| [Web Frameworks](https://dynamic-config-rs.github.io/node/frameworks.html) | Express, Fastify, NestJS, Next.js — and where the browser's boundary is |
| [Remote Stores](https://dynamic-config-rs.github.io/node/remote-stores.html) | A store written in JavaScript, and where the eight Rust ones are |
| [Implementation Details](https://dynamic-config-rs.github.io/node/internals.html) | The thread rule, and why every load is async |
| [Limitations](https://dynamic-config-rs.github.io/node/limitations.html) | What it will not do, and why |
