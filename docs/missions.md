# Working in missions

A mission holds the branches and discussions for a discrete piece of work. For example, create a **Development** topic with slug `dev`, then a mission called **Explain failed payments at checkout**. Rally allocates its number and opens its main thread. You can begin discussing the problem before attaching a repository. The mission manifest lives in PostgreSQL.

## Conversations

Post a message, mention a teammate, or address an agent with `@`. Confirming a new participant gives them access to the whole mission, including earlier conversations. You can also grant access in the mission details panel. Owners can create groups in Settings and manage their membership. The organisation owner has access to every mission.

Use the thread selector to open a separate discussion about the same task. The main conversation keeps its place even if renamed. External sessions contain transcripts from local clients. Continue those sessions in their original client. A thread reference can include spaces and colons, for example `#dev-12:Payment errors: wording`. Quote references in the shell. Thread IDs remain usable after a rename.

An idle agent in a one-human, one-agent conversation receives ordinary messages automatically. In other conversations, explicitly mention each agent you want. Mentioning a working agent steers its current invocation. On desktop, the activity pane shows streamed text and questions. On a phone, tap the activity under the addressing message. Any mission member can steer, answer, or stop the run. Other messages stay in the conversation until the agent is next addressed.

## Bringing context across missions

Use Search or `rally search` to retrieve decisions from hosted, external, and working conversations. Add a mission or thread reference using the composer. The agent receives the identifier and can fetch the original Markdown. Linking a source gives the dispatch access to the linked content. Replies may disclose it to **every member of the destination mission**. The source mission keeps its own membership.

A hosted run searches the missions its dispatcher can read. When another person steers that active run, subsequent broad searches use the intersection of the participating users' access. An explicitly linked source remains available to the run. Material already delivered to the harness can appear in later replies in that thread. A new dispatch uses its own dispatcher's retrieval authority. Roles can post from their active invocation to the hosted conversation that addressed them.

## Git branches and recovery

In mission details, add a short checkout name, remote and branch. Each repository is a child of the mission root. Rally fetches periodically and on **Refresh**. Clean, idle checkouts advance by fast-forward. A running agent keeps its working tree while a safe update waits. A changed, divergent, or conflicted checkout keeps its local contents and reports its state above the composer. Multiple agents in a mission share writable files. Fork the mission for independent branches and workspaces.

For ordinary changes, commit and push. Include “Push immediately after committing unless asked to keep the branch local” in role instructions when appropriate. **WIP commit** can generate a subject using eight recent messages, changed paths and a bounded diff. A normal WIP commit stays local until pushed.

When upstream has moved, use **WIP on new branch** to publish the local work under `rally/wip/…`. If the work is already committed locally, it publishes those commits directly. Fetch that branch, resolve the work against the original branch locally and push the result. Refresh in Rally. Once the original upstream contains both recovery tips and the workspace is clean and idle, Rally returns to the original branch and removes the recovery branch it created. A squash or cherry-pick changes commit identity. After verifying the result yourself, use **Return to original** or `rally repo return`. The explicit return preserves the recovery branch for inspection. You can also ask an agent to resolve an actual merge in the workspace.

## Inbox and devices

The inbox records direct mentions, agent questions and outcomes for work you dispatched. Use the bell in a thread to follow all messages or mute ordinary activity. Marking items read and thread subscriptions follow your account. Open an inbox item to return to its thread or working pane.

On a secure origin, install Rally through the browser's PWA menu. In Settings, enable push separately on each device and accept the browser permission dialog. The operator must configure VAPID for background notifications.

Use the push preferences in Settings to choose mentions, questions, outcomes and followed threads across all your subscribed devices. The inbox remains available when device delivery is disabled.

## Mission statuses and administrators

Every mission starts in Open. Use the status picker beneath the mission title to move it through your workflow. The owner or an admin can create, rename, recolor and remove unused statuses in Settings. Renaming a status keeps existing assignments. Missions in Done remain accessible in the sidebar and search.

The owner can invite a local user as an admin or promote a signed-in OIDC user in Settings under **Account roles**. An admin can read and manage every mission and edit workspace resources. Only the owner can invite human users and grant or remove admin access. Demoting an admin restores their ordinary membership and group grants.
