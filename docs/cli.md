# Rally CLI

The `rally` command connects your local Git workspaces, agent sessions and scripts to a self-hosted Rally server. Install the release candidate on Linux or macOS with `cargo install rally-cli --locked --version '=0.1.0-rc.1'`. You can also download a prebuilt binary from GitHub Releases or build from a checkout with `cargo install --path . --locked`.

Use `rally --help` and `rally <command> --help` for flags. Add `--json` for machine-readable output.

## Sign in and open a mission

```sh
rally login --url https://rally.example.org --oidc
rally whoami
rally mission list
rally config set --key directory --value "$HOME/rally"
eval "$(rally shell-init bash)"
rally open '#dev-12'
```

Password login accepts `--email` and prompts for the password. Use `login --oidc --no-browser` to display the sign-in URL. The browser must be able to reach the CLI's loopback callback. For automation, supply `RALLY_URL` and `RALLY_TOKEN`. An owner issues a CLI-only identity with `rally account service --name release-bot`, then grants the returned principal mission or group membership. List token IDs with `rally token list` and revoke one with `rally token revoke --id ID`.

The example creates `~/rally/dev-12` and changes into it. For Zsh use `rally shell-init zsh`. For Fish use `rally shell-init fish | source`. Without a shell function, `open` prints the command to enter the new directory. Without a configured directory, run it from an empty directory. Local Git uses your existing credentials. Use `rally sync` to refresh hosted status, local branch state and skills. A conflicting checkout is preserved.

The mission root contains a marker, skill bundles and zero or more child checkouts. `rally path '#dev-12'` prints its location. Delete or move the root to retire it locally. `rally mission list --local` prunes stale registrations. A new profile has its own URL, token and `missions.json`. For example, add these aliases and configure each profile's directory once:

```sh
alias work-rally='RALLY_CONFIG_DIR="$HOME/.config/rally-work" rally'
alias work-codex='RALLY_CONFIG_DIR="$HOME/.config/rally-work" codex'
```

## Connect through an authenticated gateway

Add `http_headers` to the existing CLI profile file, usually `~/.config/rally/config.json` or `$RALLY_CONFIG_DIR/config.json`. Keep the existing profile settings and saved Rally token. Each key is a server origin (scheme, hostname and optional port). For example, this profile obtains an employee credential from a local helper and a second header from the environment.

```json
{
  "url": "https://rally.example.org",
  "http_headers": {
    "https://rally.example.org": {
      "X-Employee-Token": {
        "command": ["gateway-cli", "token", "https://rally.example.org"],
        "login_command": ["gateway-cli", "login", "https://rally.example.org"]
      },
      "X-Gateway-Client": { "env": "COMPANY_GATEWAY_CLIENT" }
    }
  }
}
```

Choose the header names your gateway accepts. For example, for a gateway accepting `Cf-Access-Jwt-Assertion`, set that header's `command` to `["cloudflared", "access", "token", "--app", "https://rally.example.org"]` and its optional `login_command` to `["cloudflared", "access", "login", "https://rally.example.org"]`. Use the URL of your Rally Access application. Then run `rally login --oidc` and `rally whoami`. The browser uses its own sign-in session. CLI requests, including the OIDC exchange and external-session uploads, use the configured headers alongside Rally authentication.

Commands are executable and argument arrays, run directly from the profile directory. A successful lookup prints one nonempty header value to stdout. The default `timeout_ms` is 15000. Each CLI process reuses its result for `refresh_interval_ms` (default 60000) and refreshes on the next request after that interval. Put either setting alongside `command` in the header definition. An `env` source reads the named environment variable from the CLI process.

Only an explicit `rally login` with a terminal on stdin can invoke `login_command`. If lookup exits unsuccessfully or prints an empty value, Rally runs the browser-login helper with a three-minute deadline and retries lookup once. Login helpers can open a browser and display instructions on stderr. A lookup timeout returns an error immediately. When a background hook fails to authenticate, the capture stays queued. After signing in, run `rally hooks flush` to send it.

Header values stay in process memory. Cross-origin redirects stop before the next request. Git and browser traffic use their own credentials.

## Find and export conversations

```sh
rally search 'payment declined'
rally search 'why retry?' --mission dev-12
rally thread list --mission dev-12
rally read '#dev-12:Checkout wording'
rally pull '#dev-12:Checkout wording' --output wording.yaml
rally pull '#dev-12' --output ./transcripts
```

A single thread export contains its title, mission and a list of source/message pairs with known commit SHAs. A mission export writes a YAML file per thread to the supplied directory. Search returns conversation hits. Use Git to inspect source files.

## Post and dispatch

```sh
rally agent create --name reviewer --model gpt-6.2-sol --file reviewer.md
rally post --thread '#dev-12' --mention reviewer --confirm --body 'Review the retry logic.'
rally post --thread '#dev-12' --link '#dev-5:Provider discussion' --mention reviewer --file follow-up.md
rally run list --mission dev-12
rally run follow --run RUN_ID
rally run steer --run RUN_ID --body 'Focus on declined payments.'
```

Use `--silent` on a routine post to deliver the addressing message without adding it to the shared transcript. Questions and agent outcomes still appear.

Use `--mention` and `--link` to send structured references from scripts. The first mention inviting a new principal requires `--confirm`. A retryable script can pass the same `--key` for each attempt to deliver a given event. Inside a mission root or its checkout many commands infer the mission. Hosted workers also inherit the current thread.

| Commands | What they change or read |
| --- | --- |
| `topic list/create`, `mission list/show/create/update/fork` | Organise tasks. `mission update dev-12 --title … --description …` edits its descriptive fields. Use `rally status list` to find a status ID and `rally mission update dev-12 --status ID` to apply it. |
| `repo add/list/status/remove/takeover/withdraw/push/return`, `sync`, `wip` | Manage branch bindings and hosted changes. Use `wip --mission dev-12 --name project --fork` for a recovery branch. |
| `thread create/rename/subscribe/mute`, `inbox list/read` | Manage conversations and subscriptions. |
| `status list/create/update/remove` | List workflow statuses or manage their display names and colors. |
| `account invite --email … --role admin`, `account role --person ID --role member` | Invite a local admin or change an existing human account’s role as organisation owner. |
| `people list`, `member add/remove/list`, `group create/add/remove/list` | Manage access by individual or group IDs. |
| `agent list/show/create/update`, `skill create/edit/import/reconnect/attach/detach/list/show` | Manage roles, instructions, models and reusable skills. |
| `routine create/update/show/list/trigger/enable/disable/transfer` | Run and maintain cron and webhook scripts. |
| `run show/follow/steer/answer/stop` | Watch and interact with hosted work. `--answer` accepts a JSON object keyed by question ID. |

Use Settings or `rally connection create` to register personal HTTPS Git accounts. [Git connections and delegation](https://github.com/arctic-hen7/rally/blob/main/docs/git-connections.md)

## Capture local sessions

```sh
rally hooks install codex
rally hooks install claude
rally hooks status codex
```

Ensure the CLI is on the harness's PATH. Review the installed hooks in Codex's `/hooks` UI when prompted. Start the native client beneath a root opened with the same Rally profile. Prompts and replies appear under **External sessions**. Hooks check the registry and marker before recording anything. Capture uses a local outbox with replay protection. Run `rally hooks flush` after restoring connectivity or logging in again. Use `rally hooks uninstall codex` or `claude` to remove the integration.

Opening or syncing a mission installs its Rally skill at the root and links skills under `.agents/skills` and `.claude/skills` in each checkout. This lets a local agent search earlier missions with your CLI identity.

## Synchronise mission secrets

Opening a mission creates a private `.rally.env` at its root, with comments grouping values by source, and a local checksum baseline under `.rally/`. Rally adds ignore entries at that root. Reopening preserves local edits.

```sh
rally secrets pull --mission dev-12
# Edit .rally.env
rally secrets push --mission dev-12
```

Pull replaces local values and checksums. Push compares individual edited keys against their baseline. It creates new keys on the mission. Editing an inherited key creates a mission override. Deleting a mission override reveals the skill value again. If another editor changed the same key, the batch stops before writing. Save any local changes you want to keep before pulling, or pass `--overwrite KEY` for each deliberate replacement. Values are stored as data, with no shell expansion.

## Manage routines and webhooks

```sh
rally hook create --name gitea
rally hook token --hook HOOK_ID --name pull-requests
rally routine create --name review --command './review.sh' --triggers '[{"webhook":"HOOK_ID"},{"cron":"0 9 * * MON-FRI","timezone":"Australia/Sydney"}]'
rally hook show --hook HOOK_ID
rally hook revoke --hook HOOK_ID --token SENDER_ID
rally routine transfer-preview --routine ROUTINE_ID
```

Use `rally resource privacy --target ID` to inspect invitations and attachments. Pass `--private true` to restrict new attachments or `--invites '["USER_ID","GROUP_ID"]'` to replace the invite list. Remove a direct mission grant with `rally member remove --mission dev-12 --principal ID`. The response identifies remaining routes of access.
