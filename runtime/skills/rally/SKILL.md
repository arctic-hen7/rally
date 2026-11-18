---
name: rally
description: Find and read prior mission conversations, retrieve decisions, and address participants in the current Rally workspace.
---

Use `rally whoami` to identify the active principal. Read the workspace mission with `rally mission show`. Search with `rally search "decision or idea"` or limit results with `--mission dev-123`. Follow a source using `rally read '#dev-123:Thread title'`. Quote the source reference in your response.

Use `rally thread list --mission dev-123` to discover related threads. Use `rally pull '#dev-123:Thread title'` for a YAML transcript containing Markdown messages. Reference another agent using `rally post --thread THREAD --mention AGENT --body TEXT`. Only address another agent when the human requested collaboration. Mission members see retrieved context shared here.

Push after committing in most cases, unless the user requested that changes stay local. Inspect the branch and working directory before changing repositories.
