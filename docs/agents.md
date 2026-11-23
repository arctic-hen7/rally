# Agents, skills and routines

## Define a role

In Agents, create a handle, select a model supported by your provider and write developer instructions. Give roles the context that makes them useful across tasks. For example, a reviewer can carry the team's compatibility policy and use the same editing and research tools as an implementer. The creator, workspace admins and organisation owner can edit a role. Changes apply to newly dispatched invocations.

Mention a role in a hosted thread to start it. An agent has one persistent native harness session per thread. Rally delivers unseen messages in order followed by the full addressing message. When the unseen history exceeds 100,000 characters the worker receives a file reference. An interrupted active session resumes with “We got interrupted, continue”. The working pane carries questions, steering and streamed output. Its final reply goes back into the parent conversation.

## Configure the shared environment

Release installations use the bundled worker image. To add system tools such as `tea`, `mise`, or project compilers, build an image derived from `ghcr.io/arctic-hen7/rally-runtime:<release-tag>`, set `RALLY_RUNTIME_IMAGE` in the installation's `.env`, and recreate the server. The source checkout also supports editing the worker Containerfile in `./runtime` or selecting your own build context with `RALLY_RUNTIME_DIR`. Repositories may carry their own tool versions, scripts and skills.

Keep secret files outside the build context. Pass host paths in `RALLY_RUNTIME_MOUNTS` as a JSON array of Podman bind specifications and environment variables in `RALLY_RUNTIME_ENV` as a JSON object. These settings apply to agents and arbitrary routine scripts. Everyone who can author a routine must be trusted with any credentials available inside the shared environment. Use personal Git connections for hosted repository access. Operator-managed secret stores can deliver the mounted files. The engine socket belongs only in the server.

Podman provides the hosted execution boundary. Inside that container the bundled harness can run commands without a second OS sandbox or interactive approval. Hosts using the development-only local engine retain the native workspace sandbox.

The bundled default adapter uses Codex app-server over stdio. Set `RALLY_ADAPTER=acp` to use an ACP executable named by `RALLY_ACP` (default `codex-acp`), installed in a derived worker image. A replacement ACP executable must support session creation, prompts and session loading for continuity. Steering support depends on adapter capabilities. The local Codex and Claude Code capture hooks operate independently of this choice.

For a Responses-compatible provider, create an operator-owned Codex configuration like:

```toml
model_provider = "team"
[model_providers.team]
name = "Team gateway"
base_url = "https://gateway.example.org/v1"
env_key = "TEAM_GATEWAY_TOKEN"
wire_api = "responses"
```

Select this file with `--codex-config` or `RALLY_CODEX_CONFIG`, enable bypass mode, and put `TEAM_GATEWAY_TOKEN` in the provider dotenv file described in the installation guide. Each role chooses its own model ID. New roles start with `gpt-6.2-sol`. Owners and admins can change the organisation’s WIP summary model in **Settings → Models** or with `rally models settings --model MODEL`. It defaults to `gpt-6-luna`.

Rally starts a container for each hosted invocation, bind-mounts the mission directory and session state, and removes the container when the invocation finishes. Concurrent threads share checkout files. Each new process receives the current secret values. A running process, including one receiving steer messages, keeps its launch environment. A routine runs in its own container and temporary workspace.

## Private resources and scoped secrets

In an agent or skill’s **Access** section, choose **Make private** and invite individual people or groups. The creator and workspace administrators can manage access and secrets. Existing attachments survive a change to private. Inviting a private agent permits everyone in that mission to use it. Confirm the audience in the composer when you invite it. Attaching a private skill to an organisation-wide agent exposes that skill and its capabilities to everyone who can use the agent. The skill’s Access section lists its attachments. A hosted agent still searches with the authority of its dispatcher.

Open **Manage secrets** on a mission, skill or agent to add, reveal, edit or delete environment variables. A mission member can read and edit that mission’s values. A skill contributes its values wherever it is attached. Direct mission or agent definitions override their own attached skills. If two skills on the same target supply the same key, add an explicit override on the target or detach a skill. At launch the agent environment overlays the mission environment. Agent-only secrets stay within that invocation’s container. The server stores encrypted values and checksums of current values. Local workspaces receive the effective mission values through the CLI.

## Share skills

In Skills, choose **New skill**, give it a name and description, and write its instructions in the Markdown editor. Use **Preview** to check formatting. Rally combines the metadata and instructions into a workspace-local `SKILL.md`. From the CLI use `rally skill create --name reviewer --description "Review proposed changes" --file review.md`. You can also import a Git remote, ref and optional subdirectory containing `SKILL.md`. Rally fetches the remote on the server, copies regular files and executable scripts, and periodically checks for new revisions. Attach the skill to a mission or a role by name in the GUI, or by ID with the CLI:

```sh
rally skill import --name release-check --remote https://git.example.org/team/skills.git --ref main --path release-check
rally skill attach --skill SKILL_ID --target dev-12
```

An active invocation holds its skill snapshot, including across an interruption. The next invocation gets the latest successfully imported content. Local workspaces refresh their links on open or sync.

## Build a routine

Create a routine with a name, shell command, optional repositories, and any combination of schedule and webhook triggers. Use the daily or weekdays time picker, select an IANA time zone, or enter a custom cron expression. The form previews the next runs. Choose a working directory relative to the workspace. For example, add a checkout named `automation` on `main`, use `automation` as the working directory and run `./review-event.sh`. Each trigger gets a fresh workspace and a JSON event file at `$RALLY_EVENT`. A five-field cron runs on minute boundaries. The six-field form includes seconds. Under **Routines → Webhooks**, create an endpoint, then generate a named bearer token for each sending service. Copy the token when it is displayed. Add the endpoint as a trigger on one or more of your routines. A sender posts a JSON body to the displayed `/hooks/ID` URL with `Authorization: Bearer TOKEN`. An optional `Idempotency-Key` deduplicates deliveries per sender token. An accepted delivery queues each enabled subscriber once. Revoking a token leaves other senders connected.

Only the webhook owner manages its tokens and subscriptions. When the organisation owner transfers a routine, the transfer preview lists its webhook triggers. Shared endpoints stay with the original owner and disconnect from that routine. An exclusive endpoint can transfer with its existing sender tokens when explicitly selected for handover. Schedule triggers follow the routine.

A script can inspect a PR payload and run a command like this after resolving the mission and agent from its own mapping:

```sh
rally post --thread "$mission" --mention "$agent" --key "$event_id" --body "$review_message"
```

Invite the role beforehand or deliberately pass `--confirm`. Rally attributes the message to the routine. The new agent invocation continues independently as soon as the post succeeds. A routine runs with its owner's authority. Its owner can inspect recent jobs and output. The organisation owner can transfer a routine given its ID. On shutdown a running routine receives a termination signal and has ten seconds to exit. Its script is not replayed. The independently dispatched mission run resumes normally.
