import React, { useEffect, useState } from 'react';
import {
    Bell,
    Bot,
    Boxes,
    ChevronRight,
    Clock3,
    Code2,
    GitBranch,
    KeyRound,
    Library,
    Link,
    Play,
    Plus,
    RefreshCw,
    Users,
    Webhook,
    X,
} from 'lucide-react';
import { api, Avatar, Body, choice, Data, Form, IconButton, options, required, Section } from './ui';

const skillFields = [
    required('name', 'Skill name'),
    required('description', 'Description'),
    required('body', 'Instructions', 'markdown'),
];
const accountRoles = [
    { value: 'member', label: 'Member' },
    { value: 'admin', label: 'Admin' },
];
const statusFields = [
    required('name', 'Status name'),
    {
        key: 'color',
        label: 'Color',
        type: 'swatches',
        options: ['slate', 'blue', 'teal', 'amber', 'violet', 'rose', 'green'].map((value) => ({
            value,
            label: value[0].toUpperCase() + value.slice(1),
        })),
    },
];
export const roleFields = [
    required('name', 'Agent handle'),
    required('model', 'Model'),
    required('instructions', 'Developer instructions', 'area'),
];
export const routineFields = [
    required('name', 'Routine name'),
    required('command', 'Shell command', 'area'),
    { key: 'directory', label: 'Working directory', placeholder: '.' },
    { key: 'cron', label: 'Cron schedule (UTC)', placeholder: 'Leave empty for webhook only' },
    {
        key: 'repos',
        label: 'Workspace repositories (JSON)',
        type: 'json',
        placeholder: '[{"name":"scripts","remote":"…","branch":"main"}]',
    },
];
export function Manage({ page, data, edit, act, user, revision, selectMission, exit }: any) {
    const [selected, setSelected] = useState(''),
        [details, setDetails] = useState<any>(null),
        [error, setError] = useState('');
    const rows =
        page === 'Agents'
            ? data.agents
            : page === 'Skills'
              ? data.skills
              : page === 'Routines'
                ? data.routines
                : data.groups;
    useEffect(() => {
        setSelected('');
        setDetails(null);
    }, [page]);
    useEffect(() => {
        let current = true;
        if (selected) {
            const resource = { Agents: 'agent', Skills: 'skill', Routines: 'routine' }[page];
            if (resource)
                api(resource + '.show', { [resource]: selected })
                    .then((r) => {
                        if (current) setDetails(r);
                    })
                    .catch((e) => setError(e.message));
        }
        return () => {
            current = false;
        };
    }, [page, selected, revision]);
    const add = () => {
        if (page === 'Agents')
            edit({
                title: 'New agent',
                op: 'agent.create',
                fields: roleFields,
                initial: { model: 'gpt-6.2-sol' },
                submit: 'Create agent',
            });
        if (page === 'Skills')
            edit({
                title: 'New skill',
                op: 'skill.create',
                fields: skillFields,
                submit: 'Create skill',
            });
        if (page === 'Routines')
            edit({
                title: 'New routine',
                op: 'routine.create',
                fields: routineFields,
                initial: { repos: '[]' },
                reveal: 'webhook',
                submit: 'Create routine',
            });
    };
    const [preferences, setPreferences] = useState<Record<string, boolean>>({});
    useEffect(() => {
        if (page === 'Settings')
            api('inbox.settings')
                .then(setPreferences)
                .catch((e) => setError(e.message));
    }, [page]);
    async function push() {
        try {
            const reg = await navigator.serviceWorker.ready;
            const existing = await reg.pushManager.getSubscription();
            if (existing) {
                await api('device.unsubscribe', { endpoint: existing.endpoint });
                await existing.unsubscribe();
                setError('Push disabled on this device.');
                return;
            }
            const { key } = await api('device.key');
            if (!key) throw Error('Set the server VAPID keys to enable push notifications.');
            const permission = await Notification.requestPermission();
            if (permission !== 'granted') throw Error('Push permission was not granted.');
            const raw = atob(key.replace(/-/g, '+').replace(/_/g, '/'));
            const bytes = Uint8Array.from(raw, (c) => c.charCodeAt(0));
            const sub = await reg.pushManager.subscribe({
                userVisibleOnly: true,
                applicationServerKey: bytes,
            });
            await api('device.subscribe', { subscription: sub.toJSON() });
            setError('Push enabled on this device.');
        } catch (e: any) {
            setError(e.message);
        }
    }
    if (page === 'Settings')
        return (
            <div className="page">
                <h1>Settings</h1>
                <Section title="Account">
                    <div className="setting-row">
                        <Avatar name={user.name} />
                        <div>
                            <b>{user.name}</b>
                            <small>
                                {user.owner ? 'Organisation owner' : user.admin ? 'Admin' : 'Member'}
                            </small>
                        </div>
                    </div>
                    <button
                        onClick={() =>
                            edit({
                                title: 'Create CLI token',
                                op: 'token.create',
                                fields: [required('name', 'Token name')],
                                reveal: 'token',
                            })
                        }
                    >
                        <KeyRound />
                        Create CLI token
                    </button>
                    <button onClick={push}>
                        <Bell />
                        Toggle push on this device
                    </button>
                    <button onClick={exit}>Sign out</button>
                    {error && <p role="status">{error}</p>}
                </Section>
                <Section title="Push notifications">
                    {Object.entries({
                        mentions: 'Direct mentions',
                        questions: 'Agent questions',
                        outcomes: 'Agent outcomes',
                        following: 'Followed threads',
                    }).map(([key, label]) => (
                        <label key={key} className="push-preference">
                            <input
                                type="checkbox"
                                checked={preferences[key] ?? true}
                                onChange={async (event) => {
                                    try {
                                        setPreferences(
                                            await api('inbox.settings', {
                                                settings: { [key]: event.target.checked },
                                            }),
                                        );
                                    } catch (e: any) {
                                        setError(e.message);
                                    }
                                }}
                            />
                            {label}
                        </label>
                    ))}
                    <small>Applies to all your subscribed devices.</small>
                </Section>
                {(user.owner || user.admin) && (
                    <Section
                        title="Mission statuses"
                        action={
                            <button
                                onClick={() =>
                                    edit({
                                        title: 'New mission status',
                                        op: 'status.create',
                                        fields: statusFields,
                                        submit: 'Create status',
                                    })
                                }
                            >
                                <Plus />
                                New status
                            </button>
                        }
                    >
                        {data.statuses.map((status: any) => (
                            <div className="setting-line" key={status.id}>
                                <i className={'status-dot ' + status.color} />
                                <strong>{status.name}</strong>
                                {status.id === 'open' && <small>Initial status</small>}
                                <span className="spacer" />
                                <button
                                    onClick={() =>
                                        edit({
                                            title: 'Edit mission status',
                                            op: 'status.update',
                                            initial: {
                                                status: status.id,
                                                name: status.name,
                                                color: status.color,
                                            },
                                            fields: statusFields,
                                        })
                                    }
                                >
                                    Edit
                                </button>
                                {status.id !== 'open' && (
                                    <button onClick={() => act('status.remove', { status: status.id })}>
                                        Remove
                                    </button>
                                )}
                            </div>
                        ))}
                    </Section>
                )}
                {user.owner && (
                    <Section title="Account roles">
                        {data.people
                            .filter((p: any) => p.kind === 'user')
                            .map((p: any) => (
                                <div key={p.id} className="setting-line">
                                    <Avatar name={p.name} small />
                                    <strong>{p.name}</strong>
                                    <span className="spacer" />
                                    {p.owner ? (
                                        <small>Organisation owner</small>
                                    ) : (
                                        <select
                                            aria-label={'Role for ' + p.name}
                                            value={p.admin ? 'admin' : 'member'}
                                            onChange={(e) =>
                                                act('account.role', { person: p.id, role: e.target.value })
                                            }
                                        >
                                            {accountRoles.map((r) => (
                                                <option key={r.value} value={r.value}>
                                                    {r.label}
                                                </option>
                                            ))}
                                        </select>
                                    )}
                                </div>
                            ))}
                    </Section>
                )}
                {(user.owner || user.admin) && (
                    <Section title="Organisation">
                        <div className="button-wrap">
                            {user.owner && (
                                <button
                                    onClick={() =>
                                        edit({
                                            title: 'Invite a teammate',
                                            op: 'account.invite',
                                            fields: [
                                                required('email', 'Email', 'email'),
                                                choice('role', 'Account role', accountRoles),
                                            ],
                                            reveal: 'code',
                                        })
                                    }
                                >
                                    Invite user
                                </button>
                            )}
                            <button
                                onClick={() =>
                                    edit({
                                        title: 'Create service account',
                                        op: 'account.service',
                                        fields: [required('name', 'Account name')],
                                        reveal: 'token',
                                    })
                                }
                            >
                                New service account
                            </button>
                            <button
                                onClick={() =>
                                    edit({
                                        title: 'Create group',
                                        op: 'group.create',
                                        fields: [required('name', 'Group name')],
                                    })
                                }
                            >
                                New group
                            </button>
                        </div>
                        {data.groups.map((g: any) => (
                            <div className="list-row" key={g.id}>
                                <Users />
                                <div>
                                    <b>{g.name}</b>
                                    <small>{g.members.length} members</small>
                                </div>
                                <button
                                    onClick={() =>
                                        edit({
                                            title: 'Add group member',
                                            op: 'group.add',
                                            initial: { group: g.id },
                                            fields: [choice('person', 'User', options(data.people))],
                                        })
                                    }
                                >
                                    Add
                                </button>
                                {g.members.length > 0 && (
                                    <button
                                        onClick={() =>
                                            edit({
                                                title: 'Remove group member',
                                                op: 'group.remove',
                                                initial: { group: g.id },
                                                fields: [
                                                    choice(
                                                        'person',
                                                        'User',
                                                        options(
                                                            data.people.filter((p: any) =>
                                                                g.members.includes(p.id),
                                                            ),
                                                        ),
                                                    ),
                                                ],
                                            })
                                        }
                                    >
                                        Remove
                                    </button>
                                )}
                            </div>
                        ))}
                    </Section>
                )}
            </div>
        );
    const icon = page === 'Agents' ? <Bot /> : page === 'Skills' ? <Library /> : <Webhook />;
    return (
        <div className="management">
            <main className="page">
                <div className="page-title">
                    <h1>{page}</h1>
                    <div className="button-wrap">
                        {page === 'Skills' && (
                            <button
                                onClick={() =>
                                    edit({
                                        title: 'Import skill from Git',
                                        op: 'skill.import',
                                        fields: [
                                            required('name', 'Skill name'),
                                            required('remote', 'Git remote'),
                                            { key: 'ref', label: 'Branch or ref' },
                                            { key: 'path', label: 'Skill directory' },
                                        ],
                                    })
                                }
                            >
                                <GitBranch />
                                Import
                            </button>
                        )}
                        <button className="primary" onClick={add}>
                            <Plus />
                            {page === 'Agents'
                                ? 'New agent'
                                : page === 'Skills'
                                  ? 'New skill'
                                  : 'New routine'}
                        </button>
                    </div>
                </div>
                <div className="resource-list">
                    {rows.map((r: any) => (
                        <button
                            key={r.id}
                            className={'resource-row ' + (selected === r.id ? 'selected' : '')}
                            onClick={() => setSelected(r.id)}
                        >
                            <span className="resource-icon">{icon}</span>
                            <span>
                                <b>{r.name}</b>
                                <small>
                                    {r.model ||
                                        r.description ||
                                        r.remote ||
                                        r.cron ||
                                        (page === 'Skills' ? 'Workspace skill' : 'Webhook')}
                                </small>
                            </span>
                            <ChevronRight size={15} />
                        </button>
                    ))}
                    {!rows.length && (
                        <div className="empty">
                            <span className="empty-icon">{icon}</span>
                            <h3>No {page.toLowerCase()} yet</h3>
                            <button onClick={add}>
                                Create{' '}
                                {page === 'Agents' ? 'an agent' : page === 'Skills' ? 'a skill' : 'a routine'}
                            </button>
                        </div>
                    )}
                </div>
            </main>
            {selected && details?.id === selected && (
                <aside className="side-pane">
                    <header className="pane-head">
                        {icon}
                        <b>{details.name}</b>
                        <IconButton title="Close details" onClick={() => setSelected('')}>
                            <X />
                        </IconButton>
                    </header>
                    <div className="pane-content">
                        {page === 'Agents' ? (
                            <>
                                <Section title="Model">
                                    <code>{details.model}</code>
                                </Section>
                                <Section title="Instructions">
                                    <Body text={details.instructions} />
                                </Section>
                                {(details.creator === user.id || user.owner || user.admin) && (
                                    <button
                                        onClick={() =>
                                            edit({
                                                title: 'Edit agent',
                                                op: 'agent.update',
                                                initial: { ...details, agent: selected },
                                                fields: roleFields.slice(1),
                                            })
                                        }
                                    >
                                        Edit agent
                                    </button>
                                )}
                            </>
                        ) : page === 'Skills' ? (
                            <>
                                <Section title="Skill">
                                    <p>{details.description}</p>
                                    <Body text={details.body} />
                                </Section>
                                {details.remote ? (
                                    <p className="metadata">
                                        {details.remote}
                                        <br />
                                        {details.revision?.slice(0, 12)} {details.sync_error}
                                    </p>
                                ) : (
                                    <button
                                        onClick={() =>
                                            edit({
                                                title: 'Edit skill',
                                                op: 'skill.edit',
                                                initial: {
                                                    skill: selected,
                                                    name: details.name,
                                                    description: details.description,
                                                    body: details.body,
                                                },
                                                fields: skillFields,
                                            })
                                        }
                                    >
                                        Edit skill
                                    </button>
                                )}
                                <button
                                    onClick={() =>
                                        edit({
                                            title: 'Attach skill',
                                            op: 'skill.attach',
                                            initial: { skill: selected },
                                            fields: [
                                                choice(
                                                    'target',
                                                    'Agent or mission',
                                                    options([...data.agents, ...data.missions]),
                                                ),
                                            ],
                                        })
                                    }
                                >
                                    Attach to agent or mission
                                </button>
                            </>
                        ) : (
                            <>
                                <Section title="Command">
                                    <pre>{details.command}</pre>
                                    <small>
                                        {details.cron || 'Webhook'} ·{' '}
                                        {details.enabled ? 'Enabled' : 'Disabled'}
                                    </small>
                                </Section>
                                <div className="button-wrap">
                                    <button onClick={() => act('routine.trigger', { routine: selected })}>
                                        <Play />
                                        Run now
                                    </button>
                                    <button
                                        onClick={() =>
                                            act('routine.' + (details.enabled ? 'disable' : 'enable'), {
                                                routine: selected,
                                            })
                                        }
                                    >
                                        {details.enabled ? 'Disable' : 'Enable'}
                                    </button>
                                    <button
                                        onClick={() =>
                                            edit({
                                                title: 'Edit routine',
                                                op: 'routine.update',
                                                initial: {
                                                    ...details,
                                                    routine: selected,
                                                    repos: JSON.stringify(details.repos, null, 2),
                                                },
                                                fields: routineFields,
                                            })
                                        }
                                    >
                                        Edit
                                    </button>
                                </div>
                                <Section title="Recent runs">
                                    {details.jobs.map((j: any) => (
                                        <details key={j.id}>
                                            <summary>
                                                {j.status} · {new Date(j.created).toLocaleString()}
                                            </summary>
                                            <pre>{j.output}</pre>
                                        </details>
                                    ))}
                                </Section>
                            </>
                        )}
                    </div>
                </aside>
            )}
        </div>
    );
}
