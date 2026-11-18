# Rally CLI

Use `rally --help` and `rally <command> --help` for flags. Add `--json` for stable machine-readable output. Install the release binary on your PATH or run `cargo install --path . --locked --bin rally`.

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
rally agent create --name reviewer --model gpt-6.2-luna --file reviewer.md
rally post --thread '#dev-12' --mention reviewer --confirm --body 'Review the retry logic.'
rally post --thread '#dev-12' --link '#dev-5:Provider discussion' --mention reviewer --file follow-up.md
rally run list --mission dev-12
rally run follow --run RUN_ID
rally run steer --run RUN_ID --body 'Focus on declined payments.'
```

Use `--mention` and `--link` to send structured references from scripts. The first mention inviting a new principal requires `--confirm`. A retryable script can pass the same `--key` for each attempt to deliver a given event. Inside a mission root or its checkout many commands infer the mission. Hosted workers also inherit the current thread.

| Commands | What they change or read |
| --- | --- |
| `topic list/create`, `mission list/show/create/update/fork/archive` | Organise tasks. `mission update dev-12 --title … --description …` edits the manifest's descriptive fields. |
| `repo add/list/status/remove/push/return`, `sync`, `wip` | Manage branch bindings and hosted changes. Use `wip --mission dev-12 --name project --fork` for a recovery branch. |
| `thread create/rename/subscribe/mute`, `inbox list/read` | Manage conversations and subscriptions. |
| `people list`, `member add/remove/list`, `group create/add/remove/list` | Manage access by individual or group IDs. |
| `agent list/show/create/update`, `skill create/edit/import/attach/detach/list/show` | Manage roles, instructions, models and reusable skills. |
| `routine create/update/show/list/trigger/enable/disable/transfer` | Run and maintain cron and webhook scripts. |
| `run show/follow/steer/answer/stop` | Watch and interact with hosted work. `--answer` accepts a JSON object keyed by question ID. |

## Capture local sessions

```sh
rally hooks install codex
rally hooks install claude
rally hooks status codex
```

Ensure the CLI is on the harness's PATH. Review the installed hooks in Codex's `/hooks` UI when prompted. Start the native client beneath a root opened with the same Rally profile. Prompts and replies appear under **External sessions**. Hooks check the registry and marker before recording anything. Capture uses a local outbox with replay protection. Run `rally hooks flush` after restoring connectivity or logging in again. Use `rally hooks uninstall codex` or `claude` to remove the integration.

Opening or syncing a mission installs its Rally skill at the root and links skills under `.agents/skills` and `.claude/skills` in each checkout. This lets a local agent search earlier missions with your CLI identity.
