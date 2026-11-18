# Agents, skills and routines

## Define a role

In Agents, create a handle, select a model supported by your provider and write developer instructions. Give roles the context that makes them useful across tasks. For example, a reviewer can carry the team's compatibility policy and use the same editing and research tools as an implementer. The creator and organisation owner can edit a role. Changes apply to newly dispatched invocations.

Mention a role in a hosted thread to start it. An agent has one persistent native harness session per thread. Rally delivers unseen messages in order followed by the full addressing message. When the unseen history exceeds 100,000 characters the worker receives a file reference. An interrupted active session resumes with “We got interrupted, continue”. The working pane carries questions, steering and streamed output. Its final reply goes back into the parent conversation.

## Configure the shared environment

Supply one operator-owned build directory containing a `Containerfile` via `RALLY_RUNTIME_DIR` in Compose. Start from the repository's runtime image definition. Add system tools such as `tea`, `mise`, project compilers or a different ACP adapter there. Recreate the server after changing runtime configuration. Repositories may carry their own tool versions, scripts and skills.

Keep secret files outside the build context. Pass host paths in `RALLY_RUNTIME_MOUNTS` as a JSON array of Podman bind specifications and environment variables in `RALLY_RUNTIME_ENV` as a JSON object. These settings apply to agents and arbitrary routine scripts. Everyone who can author a routine must be trusted with any credentials available inside the shared environment. Use narrow Git and provider credentials. Operator-managed secret stores can deliver the mounted files. The engine socket belongs only in the server.

Podman provides the hosted execution boundary. Inside that container the bundled harness can run commands without a second OS sandbox or interactive approval. Hosts using the development-only local engine retain the native workspace sandbox.

The bundled default adapter uses Codex app-server over stdio. Set `RALLY_ADAPTER=acp` to use the bundled Codex ACP bridge. A replacement ACP executable must support session creation, prompts and session loading for continuity. Steering support depends on adapter capabilities. The local Codex and Claude Code capture hooks operate independently of this choice.

For a Responses-compatible provider, create an operator-owned Codex configuration like:

```toml
model_provider = "team"
[model_providers.team]
name = "Team gateway"
base_url = "https://gateway.example.org/v1"
env_key = "TEAM_GATEWAY_TOKEN"
wire_api = "responses"
```

Set `RALLY_MODEL_PROVIDER=team` for the native adapter. Mount this file into both `/session/config.toml:ro` (hosted sessions) and `/root/.codex/config.toml:ro` (standalone commands). Supply the provider's token using the operator's runtime configuration. A preconfigured Codex profile can similarly supply the appropriate authentication file for your provider. Leave the rest of `/session` writable so Rally can resume native conversations. Each role chooses its own model ID. Set `RALLY_WIP_MODEL` for commit summaries if the default `gpt-6.2-luna` is unavailable. A WIP summary can also be entered manually.

## Share skills

Create a Markdown skill in the GUI or import a Git remote, ref and optional subdirectory containing `SKILL.md`. Rally fetches the remote on the server, copies regular files and executable scripts, and periodically checks for new revisions. Attach the skill to a mission or a role by name in the GUI, or by ID with the CLI:

```sh
rally skill import --name release-check --remote https://git.example.org/team/skills.git --ref main --path release-check
rally skill attach --skill SKILL_ID --target dev-12
```

An active invocation holds its skill snapshot, including across an interruption. The next invocation gets the latest successfully imported content. Local workspaces refresh their links on open or sync.

## Build a routine

Create a routine with a name, shell command, optional UTC cron expression and optional repository list. Choose a working directory relative to the workspace. For example, add a checkout named `automation` on `main`, use `automation` as the working directory and run `./review-event.sh`. Each trigger gets a fresh workspace and a JSON event file at `$RALLY_EVENT`. A five-field cron runs on minute boundaries. The six-field form includes seconds. A webhook accepts a JSON POST at the secret URL returned on creation.

A script can inspect a PR payload and run a command like this after resolving the mission and agent from its own mapping:

```sh
rally post --thread "$mission" --mention "$agent" --key "$event_id" --body "$review_message"
```

Invite the role beforehand or deliberately pass `--confirm`. Rally attributes the message to the routine. The new agent invocation continues independently as soon as the post succeeds. A routine runs with its owner's authority. Its owner can inspect recent jobs and output. The organisation owner can transfer a routine given its ID. On shutdown a running routine receives a termination signal and has ten seconds to exit. Its script is not replayed. The independently dispatched mission run resumes normally.
