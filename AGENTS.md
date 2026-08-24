<!-- @cf:root-agents -->
```toml
cf-studio-path = ".cf-studio"
```

ALWAYS resolve and enforce prerequisites of skills/workflows/commands BEFORE applying user intent.
<!-- /@cf:root-agents -->

These instructions are for AI assistants working in this project.

If the instruction sounds unclear, vague or requires more context. Ask for clarification.

Always open `@/guidelines/README.md` first (entry point for project-wide guidelines).

Open additional docs only when relevant:

- If the task adds/changes dependencies (Cargo.toml), introduces a new crate, involves working with 3rd-party crates (such as those for serialization/deserialization), open `@/guidelines/DEPENDENCIES.md`.

- If the task touches ToolKit/Gear architecture (Gear layout, `@/lib/toolkit*`, plugins, REST wiring, ClientHub, OpenAPI, lifecycle/stateful tasks, SSE, standardized HTTP errors), open `@/docs/toolkit_unified_system/README.md`.

## Comments

Code shows *how*; a comment carries *why* — a non-obvious constraint, a
deliberate deviation, a rejected alternative, a gotcha. Never narrate what the
next line does.

**Never write the history of the code in a comment.** No dates, no review ids,
no commit hashes, no "this paragraph said", no "until <date> this was", no "that
premise was false", no "PAID <date>". What changed and when belongs in the
commit message and in the decision register; a comment states what is true now.
A doc that describes the state of the work goes stale in the commit that
finishes it.

A rejected alternative is durable and belongs in the present tense: write what
the other shape *costs* — "a six-axis copy of the comparison here reads two
usage lines of one market as siblings" — rather than what it cost when somebody
tried it.

Citations are the exception and are durable: keep every `D-NN`, `inst-*`,
`§N.N`, ADR id and migration id. A review id is not a citation, and a citation
whose only role is to attribute a narration goes with the narration.

Never put a count in prose beside a roster in code. The roster is the answer;
the number is the only part that can go stale.
