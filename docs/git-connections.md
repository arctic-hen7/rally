# Git connections

Open **Settings → Git connections → New connection**. Enter a display name, the HTTPS forge host (including a nonstandard port if needed), the Git username expected by the forge and a personal access token. Use the repository's HTTPS clone URL. Select a token with read access for skills and read/write access for repositories you intend to push to. You can keep multiple accounts on one forge. Select **Anonymous** for public read-only access.

## Share a repository with a mission

Add repositories when creating a mission or open **Mission details → Repositories → Add repository**. Choose a checkout directory, branch and connection. Confirming delegates that repository's Git access to the mission. Every current and future member, including hosted agents, can perform the Git operations your forge account permits on that repository. Forge branch protections still apply. The initial check verifies that the chosen account can read the branch. The forge checks write permission on each push.

The hosted checkout's `origin` points to Rally. Rally looks up the repository's current connection for each Git smart HTTP request and authenticates to the configured upstream. Worker processes receive a temporary Rally credential scoped to their workspace. Rally stores forge tokens encrypted alongside other encrypted database values. You can inspect where a connection is used, replace its token or disconnect it in Settings.

The forge records pushes under the account that contributed the repository. Commit authorship is independent. Give each role instructions such as “Commit as Engineering Agent <engineering-agent@company.com> and push after committing.” Use command-local identity settings on a shared checkout:

```sh
git -c user.name='Engineering Agent' -c user.email=engineering-agent@company.com commit -m 'fix: handle expired sessions'
git push origin HEAD
```

**Take over** checks the replacement contributor's own access. **Withdraw my access** pauses remote operations and preserves files and uncommitted work. When a contributor loses effective mission membership through direct removal, group changes or admin demotion, Rally shows which repositories will need a new contributor before applying the change. Rejoining the mission still requires an explicit takeover. A fork copies repository definitions and requires fresh delegation for the new mission.

Local `rally open` clones and fetches from the upstream URL using your computer's Git configuration. If one repository is inaccessible, the CLI reports its checkout name and contributor, then continues with the others. Obtain access from the forge and rerun `rally open`.

## Skills and routines

Imported skills fetch on the server using the importer's selected connection and read-only Git operations. A disconnected connection or a forge authentication rejection pauses sync. Existing runs and attached skills keep their last successful snapshot. **Reconnect Git source** verifies the selected ref and skill directory using the replacement connection. Replacing a token on its existing connection also schedules its imports to retry on the next sync.

Select the routine creator's connections in the routine's workspace repositories. A job can use these checkouts and can create a mission or add a repository as its owner. For an exact repository URL with one declared connection, the job can omit the connection ID. Include `--confirm` when delegating authenticated access to the mission. Transferring a routine clears its authenticated repository bindings and disables scheduling until the new owner configures access and enables the routine.

When a routine posts into an existing mission, the dispatched agent runs in that mission and uses each repository's current contributor connection. Use a silent post to keep a recurring instruction out of human conversation history, exports, search and prompt notifications. The agent still receives the instruction. Its questions, replies and outcome notifications follow the usual rules.

```sh
rally post --silent --thread '#dev-12' --mention reviewer --confirm --key review-event-123 --body 'Review the new PR discussion.'
```

## CLI examples

```sh
rally connection create --name Work --host git.example.org --username alice
rally connection list
rally repo add --mission dev-12 --name app --remote https://git.example.org/team/app.git --branch main --connection CONNECTION_ID --confirm
rally connection usage --connection CONNECTION_ID
rally member remove --mission dev-12 --principal USER_ID --preview
rally repo takeover --mission dev-12 --name app --connection CONNECTION_ID --confirm
rally repo withdraw --mission dev-12 --name app
rally skill reconnect --skill SKILL_ID --connection CONNECTION_ID
```

The CLI prompts for a token on create and update. For automation, pipe the token to `--token-stdin`. To attach repositories at mission creation, pass `--repos` with a JSON array of objects containing `name`, `remote`, `branch` and an optional `connection`.
